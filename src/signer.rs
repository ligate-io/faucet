//! Hot-key signer. Holds the Ed25519 private key in memory, signs +
//! submits a `bank.transfer` to drip `$LGT` to the requested address.
//!
//! ## Status: NOT FULLY WIRED YET
//!
//! The chain-side `ligate-client` SDK does not yet expose a
//! `bank.transfer` helper, only attestation helpers (registers,
//! submits). Wiring the full submission pipeline requires either:
//!
//! 1. Adding a `transfer` helper to `ligate-client` (chain repo PR),
//!    then taking it as a `git = "..."` dep here, OR
//! 2. Hand-rolling the call-message construction here using
//!    `sov-bank` types directly (heavier dep graph).
//!
//! Both options work. (1) is preferred because the chain repo is the
//! source of truth for transaction shape; the faucet should consume
//! that shape, not duplicate it. Tracked as a follow-up to chain
//! issue #95.
//!
//! Until that lands, this module is a SCAFFOLD: it validates the
//! request shape and returns a stable shape for the HTTP layer to
//! report on, but does NOT actually submit a transaction. Operators
//! deploying this binary against a real devnet will see the faucet
//! respond with `tx_hash: "0xpending..."` placeholders.
//!
//! Tracking issue for the wiring follow-up: see chain repo #95.

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SignerError {
    /// Reserved for the failure mode the scaffolded `drip` doesn't
    /// yet return (since it always succeeds with a placeholder hash).
    /// Once the chain submission wiring lands, this variant is
    /// returned when the chain RPC rejects the transaction.
    #[allow(dead_code)]
    #[error("signer not yet wired to chain submission. See follow-up to chain issue #95.")]
    NotWired,
    #[error("invalid recipient address: {0}")]
    InvalidAddress(String),
}

#[derive(Debug, Clone)]
pub struct DripReceipt {
    /// Transaction hash returned by the chain (or a placeholder
    /// while the signer is stubbed).
    pub tx_hash: String,
    /// Drip amount in nano-LGT.
    pub amount_nano: u128,
}

pub struct Signer {
    /// Hex-encoded private key (32 bytes, 64 chars). Held in memory
    /// for the lifetime of the process. Never logged.
    _signing_key_hex: String,
    /// Public REST endpoint for the target chain.
    _chain_rpc: String,
    /// Local counter for placeholder tx hashes while the signer is
    /// stubbed. Lets the HTTP layer return distinct values per drip.
    drip_counter: AtomicU64,
}

// Manual Debug to keep the signing key out of any debug prints.
// `unwrap_err()` in tests requires `T: Debug` on the Ok side, hence
// this impl exists.
impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("signing_key_hex", &"<redacted>")
            .field("chain_rpc", &self._chain_rpc)
            .field("drip_counter", &self.drip_counter)
            .finish()
    }
}

impl Signer {
    pub fn from_hex_key(signing_key_hex: &str, chain_rpc: String) -> Result<Self> {
        // Sanity-check the hex shape. Actual ed25519 derivation
        // happens once the wiring lands.
        if signing_key_hex.len() != 64 {
            anyhow::bail!("signer key must be 64 hex chars, got {}", signing_key_hex.len());
        }
        hex::decode(signing_key_hex)?;
        Ok(Self {
            _signing_key_hex: signing_key_hex.to_string(),
            _chain_rpc: chain_rpc,
            drip_counter: AtomicU64::new(0),
        })
    }

    /// Sign + submit a `bank.transfer` of `amount_nano` from this
    /// signer to `recipient`. Returns the tx hash.
    ///
    /// SCAFFOLD: returns a placeholder hash. See module-level docs.
    pub async fn drip(&self, recipient: &str, amount_nano: u128) -> Result<DripReceipt, SignerError> {
        validate_lig_address(recipient)?;
        let n = self.drip_counter.fetch_add(1, Ordering::Relaxed);
        // Placeholder tx hash. When wired, this returns the real
        // hash from the chain RPC's submit response.
        let tx_hash = format!("0xpending{n:016x}");
        Ok(DripReceipt { tx_hash, amount_nano })
    }
}

/// Cheap shape-only check on a Ligate Bech32m address. Real
/// validation (Bech32 checksum, 28-byte payload) happens at
/// transaction-submit time when ligate-client deserializes; this
/// function just rejects obvious garbage early.
fn validate_lig_address(addr: &str) -> Result<(), SignerError> {
    if !addr.starts_with("lig1") {
        return Err(SignerError::InvalidAddress(format!(
            "expected lig1... bech32m, got {addr}"
        )));
    }
    if addr.len() < 20 || addr.len() > 90 {
        return Err(SignerError::InvalidAddress(format!(
            "address length {} outside expected range (20..90)",
            addr.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_lig_address() {
        let err = validate_lig_address("celestia1abc").unwrap_err();
        assert!(matches!(err, SignerError::InvalidAddress(_)));
    }

    #[test]
    fn rejects_too_short() {
        let err = validate_lig_address("lig1abc").unwrap_err();
        assert!(matches!(err, SignerError::InvalidAddress(_)));
    }

    #[test]
    fn accepts_well_formed() {
        validate_lig_address("lig1h72nh5c7jfjkcygku4thsh2t53dyh33kkpktpy84w06qwr4agvt").unwrap();
    }

    #[tokio::test]
    async fn drip_returns_receipt_with_distinct_hashes() {
        let key = "00".repeat(32);
        let signer = Signer::from_hex_key(&key, "http://localhost:12346".to_string()).unwrap();
        let r1 = signer
            .drip("lig1h72nh5c7jfjkcygku4thsh2t53dyh33kkpktpy84w06qwr4agvt", 1_000_000_000)
            .await
            .unwrap();
        let r2 = signer
            .drip("lig1h72nh5c7jfjkcygku4thsh2t53dyh33kkpktpy84w06qwr4agvt", 1_000_000_000)
            .await
            .unwrap();
        assert_ne!(r1.tx_hash, r2.tx_hash);
        assert_eq!(r1.amount_nano, 1_000_000_000);
    }

    #[test]
    fn from_hex_key_rejects_wrong_length() {
        let err = Signer::from_hex_key("abcd", "http://x".to_string()).unwrap_err();
        assert!(format!("{err:#}").contains("64 hex chars"));
    }
}
