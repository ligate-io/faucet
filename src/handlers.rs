//! HTTP handlers.
//!
//! Three endpoints:
//!
//! - `GET /health` always 200, for orchestrator probes.
//! - `POST /faucet { "address": "lig1..." }` rate-limited drip.
//! - `GET /faucet/status` rate-limit window, drip amount, drips so
//!   far. No auth, no per-IP info exposure.

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::ratelimit::RateCheck;
use crate::signer::SignerError;

#[derive(Debug, Deserialize)]
pub struct DripRequest {
    pub address: String,
}

#[derive(Debug, Serialize)]
pub struct DripResponse {
    pub address: String,
    pub tx_hash: String,
    pub amount_nano: u128,
    pub drip_amount_lgt: f64,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub retry_after_secs: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub drip_amount_nano: u128,
    pub drip_amount_lgt: f64,
    pub rate_limit_secs: u64,
    pub addresses_dripped: usize,
}

pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

pub async fn status(State(state): State<AppState>) -> impl IntoResponse {
    let drip_nano = state.config.drip_amount;
    Json(StatusResponse {
        drip_amount_nano: drip_nano,
        drip_amount_lgt: nano_to_lgt(drip_nano),
        rate_limit_secs: state.config.rate_limit_window().as_secs(),
        addresses_dripped: state.rate_limiter.drip_count(),
    })
}

pub async fn drip(
    State(state): State<AppState>,
    Json(req): Json<DripRequest>,
) -> Result<Json<DripResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 1. Rate-limit check BEFORE we touch the signer. Avoids
    //    spending a potential drip on an over-cap address.
    match state.rate_limiter.check(&req.address) {
        RateCheck::Allowed => {}
        RateCheck::Blocked { retry_after } => {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorResponse {
                    error: format!(
                        "address rate-limited; retry in {} seconds",
                        retry_after.as_secs()
                    ),
                    retry_after_secs: Some(retry_after.as_secs()),
                }),
            ));
        }
    }

    // 2. Sign + submit. SCAFFOLD: see signer module docs.
    let receipt = state
        .signer
        .drip(&req.address, state.config.drip_amount)
        .await
        .map_err(|e| match e {
            SignerError::InvalidAddress(msg) => (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error: msg, retry_after_secs: None }),
            ),
            SignerError::NotWired => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: e.to_string(),
                    retry_after_secs: None,
                }),
            ),
        })?;

    // 3. Record AFTER the chain accepted (so failed submits don't
    //    consume the address's window).
    state.rate_limiter.record(&req.address);

    Ok(Json(DripResponse {
        address: req.address,
        tx_hash: receipt.tx_hash,
        amount_nano: receipt.amount_nano,
        drip_amount_lgt: nano_to_lgt(receipt.amount_nano),
    }))
}

fn nano_to_lgt(nano: u128) -> f64 {
    (nano as f64) / 1_000_000_000.0
}
