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
//! - `RUST_LOG` (default `info,ligate_faucet=info`).
//!
//! Tracking issue: <https://github.com/ligate-io/ligate-chain/issues/95>.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use axum::{routing::{get, post}, Router};
use tower_http::trace::TraceLayer;
use tracing::info;

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
    let bind: SocketAddr = config.bind.parse().context("parsing FAUCET_BIND as SocketAddr")?;

    // Parse the LGT token id from hex into the SDK's TokenId.
    let token_id_bytes = hex::decode(&config.lgt_token_id_hex)
        .context("FAUCET_LGT_TOKEN_ID must be valid hex")?;
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
    let rate_limiter = ratelimit::RateLimiter::new(config.rate_limit_window());

    let state = AppState {
        config: Arc::new(config),
        rate_limiter: Arc::new(rate_limiter),
        signer: Arc::new(signer),
    };

    let app = Router::new()
        .route("/health", get(handlers::health))
        .route("/faucet", post(handlers::drip))
        .route("/faucet/status", get(handlers::status))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    info!(?bind, "ligate-faucet starting");

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding listener at {bind}"))?;
    axum::serve(listener, app).await.context("axum serve")?;

    Ok(())
}
