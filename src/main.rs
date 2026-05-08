//! Ligate Chain devnet faucet.
//!
//! Rate-limited HTTP service that drips a fixed amount of `$LGT` to
//! addresses on a public devnet (`ligate-devnet-1`). Holds a hot
//! signer key whose address is pre-funded at genesis (separate from
//! the bootstrap / treasury key so a faucet drain doesn't touch the
//! treasury).
//!
//! Config via env (read at startup, never reloaded):
//!
//! - `FAUCET_BIND` (default `0.0.0.0:8080`): HTTP server bind address.
//! - `FAUCET_CHAIN_RPC` (required): public Ligate Chain REST endpoint
//!   (e.g. `https://rpc.ligate.io`).
//! - `FAUCET_SIGNER_KEY` (required): 64-char hex Ed25519 private key
//!   (32 bytes). The address derived from this key must hold a
//!   pre-funded `$LGT` balance.
//! - `FAUCET_DRIP_AMOUNT` (default `1000000000`): nano-LGT per drip
//!   (1 `$LGT` = 1e9 nano).
//! - `FAUCET_RATE_LIMIT_SECS` (default `86400`): cooldown per
//!   recipient address (24 hours).
//! - `FAUCET_MIN_DRIPS_BUDGET` (default `100`): startup sanity check.
//!   The faucet refuses to start if its current LGT balance covers
//!   fewer than this many drips at the configured `FAUCET_DRIP_AMOUNT`.
//!   Catches "set drip to 1000 LGT thinking it's nano" typos before
//!   they drain the hot key. Set to `0` to disable.
//! - `RUST_LOG` (default `info,ligate_faucet=info`).
//!
//! Tracking issue: <https://github.com/ligate-io/ligate-chain/issues/95>.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::{
    routing::{get, post},
    Router,
};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

mod config;
mod handlers;
mod ratelimit;
mod signer;

#[derive(Clone)]
pub(crate) struct AppState {
    pub config: Arc<config::Config>,
    pub rate_limiter: Arc<ratelimit::RateLimiter>,
    pub signer: Arc<signer::Signer>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Structured JSON logs by default. Override with RUST_LOG=debug
    // for local development.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ligate_faucet=info".into()),
        )
        .json()
        .init();

    let config = config::Config::from_env().context("loading config from env")?;
    let bind: SocketAddr = config
        .bind
        .parse()
        .context("parsing FAUCET_BIND as SocketAddr")?;

    // Parse the LGT token id from hex into the SDK's TokenId.
    let token_id_bytes =
        hex::decode(&config.lgt_token_id_hex).context("FAUCET_LGT_TOKEN_ID must be valid hex")?;
    let token_id = sov_bank::TokenId::try_from(token_id_bytes.as_slice())
        .map_err(|e| anyhow::anyhow!("FAUCET_LGT_TOKEN_ID wrong shape: {e:?}"))?;

    let signer = signer::Signer::new(
        &config.signer_key,
        config.chain_rpc.clone(),
        config.chain_id,
        config.chain_hash,
        token_id,
        config.starting_nonce,
    )
    .context("loading signer")?;

    info!(faucet_address = %signer.address(), "signer loaded");

    // Startup drip-budget sanity check.
    //
    // Catches the typo class "operator set FAUCET_DRIP_AMOUNT to whole
    // LGT instead of nano-LGT (1e9× too much) and would drain the hot
    // key in a handful of drips." Refuses to start if current balance
    // covers fewer than `FAUCET_MIN_DRIPS_BUDGET` drips at the
    // configured `FAUCET_DRIP_AMOUNT`. Default 100; set to 0 to skip.
    //
    // The chain query uses the SDK's `get_balance_for_holder` and so
    // assumes the chain at `FAUCET_CHAIN_RPC` is reachable. The
    // systemd unit's `Requires=ligate-node.service` ordering means
    // that's true on a fresh GCP VM boot. We retry briefly to handle
    // the race where the chain is reachable but hasn't indexed the
    // faucet's pre-funded balance yet.
    let min_drips_budget = std::env::var("FAUCET_MIN_DRIPS_BUDGET")
        .ok()
        .map(|s| s.parse::<u64>())
        .transpose()
        .context("FAUCET_MIN_DRIPS_BUDGET must be a non-negative integer")?
        .unwrap_or(100);

    if min_drips_budget > 0 {
        let drip_amount = config.drip_amount;
        let balance = query_balance_with_retry(&signer, 5, Duration::from_secs(2))
            .await
            .context(
                "startup drip-budget check failed; \
                 set FAUCET_MIN_DRIPS_BUDGET=0 to skip if the chain is intentionally unreachable",
            )?;
        let budget = balance / drip_amount;
        if (budget as u64) < min_drips_budget {
            anyhow::bail!(
                "signer balance ({balance} nano-LGT) covers only {budget} drips at \
                 {drip_amount} nano-LGT/drip; minimum is {min_drips_budget} \
                 (FAUCET_MIN_DRIPS_BUDGET). Either fund the signer or lower \
                 FAUCET_DRIP_AMOUNT before starting."
            );
        }
        info!(
            balance,
            drip_amount, budget, min_drips_budget, "drip-budget check OK"
        );
    } else {
        warn!("FAUCET_MIN_DRIPS_BUDGET=0 — skipping startup balance check");
    }

    let rate_limiter = ratelimit::RateLimiter::new(config.rate_limit_window());

    let state = AppState {
        config: Arc::new(config),
        rate_limiter: Arc::new(rate_limiter),
        signer: Arc::new(signer),
    };

    // Permissive CORS for v0 devnet — partner web apps (Mneme,
    // Themisra, design-partner sites) hit `faucet.ligate.io/faucet`
    // from arbitrary origins. Tighten the origin allow-list at
    // testnet+; for devnet, "anyone can hit the faucet from any
    // browser" matches the rest of the public-permissionless story.
    let cors = CorsLayer::permissive();

    let app = Router::new()
        .route("/health", get(handlers::health))
        .route("/faucet", post(handlers::drip))
        .route("/faucet/status", get(handlers::status))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state);

    info!(?bind, "ligate-faucet starting");

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding listener at {bind}"))?;
    axum::serve(listener, app).await.context("axum serve")?;

    Ok(())
}

/// Query the signer's own LGT balance with bounded retries.
///
/// On a fresh GCP VM boot the chain may be reachable but still
/// indexing genesis when the faucet starts. A short retry loop
/// avoids flapping the systemd unit through one or two restarts.
async fn query_balance_with_retry(
    signer: &signer::Signer,
    max_attempts: u32,
    backoff: Duration,
) -> anyhow::Result<u128> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..max_attempts {
        match signer.query_self_balance().await {
            Ok(b) => return Ok(b),
            Err(e) => {
                warn!(?e, attempt, "balance query failed; retrying");
                last_err = Some(e);
                tokio::time::sleep(backoff).await;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("balance query failed with no error captured")))
}
