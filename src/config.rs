//! Env-var-driven config. Read once at startup, never reloaded.
//!
//! Validation lives here: missing required vars, malformed numerics,
//! unparseable signer key all fail fast at boot rather than at
//! first-request time.

use std::time::Duration;

use anyhow::{anyhow, Context};

const DEFAULT_BIND: &str = "0.0.0.0:8080";
const DEFAULT_DRIP_AMOUNT: u128 = 1_000_000_000; // 1 $LGT in nano-LGT
const DEFAULT_RATE_LIMIT_SECS: u64 = 24 * 60 * 60; // 24h per address

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: String,
    pub chain_rpc: String,
    pub signer_key: String,
    pub drip_amount: u128,
    pub chain_id: u64,
    pub chain_hash: [u8; 32],
    pub lgt_token_id_hex: String,
    pub starting_nonce: u64,
    rate_limit_secs: u64,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind = std::env::var("FAUCET_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_string());
        let chain_rpc = std::env::var("FAUCET_CHAIN_RPC")
            .context("FAUCET_CHAIN_RPC is required (e.g. https://rpc.ligate.io)")?;
        let signer_key = std::env::var("FAUCET_SIGNER_KEY")
            .context("FAUCET_SIGNER_KEY is required (64-char hex private key)")?;
        if signer_key.len() != 64 {
            return Err(anyhow!(
                "FAUCET_SIGNER_KEY must be 64 hex chars (32 bytes), got {}",
                signer_key.len()
            ));
        }
        if hex::decode(&signer_key).is_err() {
            return Err(anyhow!("FAUCET_SIGNER_KEY must be valid hex"));
        }

        let drip_amount = std::env::var("FAUCET_DRIP_AMOUNT")
            .ok()
            .map(|s| s.parse::<u128>())
            .transpose()
            .context("FAUCET_DRIP_AMOUNT must be a non-negative integer (nano-LGT)")?
            .unwrap_or(DEFAULT_DRIP_AMOUNT);

        let rate_limit_secs = std::env::var("FAUCET_RATE_LIMIT_SECS")
            .ok()
            .map(|s| s.parse::<u64>())
            .transpose()
            .context("FAUCET_RATE_LIMIT_SECS must be a non-negative integer (seconds)")?
            .unwrap_or(DEFAULT_RATE_LIMIT_SECS);

        // Chain identity for transaction construction. These come from
        // the chain's `/v1/rollup/info` endpoint at runtime; we capture
        // them as env vars at boot for predictability. If the operator
        // re-deploys against a new chain, env-var update + restart is
        // cleaner than dynamic re-fetch.
        let chain_id = std::env::var("FAUCET_CHAIN_ID")
            .context("FAUCET_CHAIN_ID is required (numeric, from chain_state.json)")?
            .parse::<u64>()
            .context("FAUCET_CHAIN_ID must be u64")?;

        let chain_hash_hex = std::env::var("FAUCET_CHAIN_HASH")
            .context("FAUCET_CHAIN_HASH is required (64-char hex from /v1/rollup/info)")?;
        if chain_hash_hex.len() != 64 {
            return Err(anyhow!(
                "FAUCET_CHAIN_HASH must be 64 hex chars, got {}",
                chain_hash_hex.len()
            ));
        }
        let chain_hash_bytes =
            hex::decode(&chain_hash_hex).context("FAUCET_CHAIN_HASH must be valid hex")?;
        let mut chain_hash = [0u8; 32];
        chain_hash.copy_from_slice(&chain_hash_bytes);

        let lgt_token_id_hex = std::env::var("FAUCET_LGT_TOKEN_ID")
            .context("FAUCET_LGT_TOKEN_ID is required (token id hex from bank.json)")?;

        let starting_nonce = std::env::var("FAUCET_STARTING_NONCE")
            .ok()
            .map(|s| s.parse::<u64>())
            .transpose()
            .context("FAUCET_STARTING_NONCE must be u64")?
            .unwrap_or(0);

        Ok(Self {
            bind,
            chain_rpc,
            signer_key,
            drip_amount,
            chain_id,
            chain_hash,
            lgt_token_id_hex,
            starting_nonce,
            rate_limit_secs,
        })
    }

    pub fn rate_limit_window(&self) -> Duration {
        Duration::from_secs(self.rate_limit_secs)
    }
}
