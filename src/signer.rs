//! Hot-key signer for ligate-faucet.
//!
//! Signs and submits a `bank.transfer` to drip `$LGT` to the
//! requested address. Uses [`ligate_client::submit::Submitter`] for
//! chain interaction.
//!
//! ## Wire format (for context)
//!
//! 1. Build `RuntimeCall::Bank(CallMessage::Transfer { to, coins })`
//!    against the chain's runtime composition.
//! 2. Wrap in `UnsignedTransaction::new` with chain id, nonce, fees.
//! 3. Sign: `unsigned.sign(&private_key, &chain_hash)` returns a
//!    `Transaction`. The signature binds to `chain_hash` so the same
//!    private key produces a different signature on each chain id.
//! 4. Borsh-encode the signed transaction. The chain's
//!    `POST /v1/sequencer/txs` handler wraps the body in
//!    `AuthenticatorInput::Standard(RawTx { data })` server-side, so
//!    we do NOT pre-wrap on the client. (Doing so double-wraps and
//!    the chain rejects with `Cannot decompress Edwards point`.
//!    See `ligate-chain#245`.)
//! 5. Submit via `Submitter::submit_raw_tx`.
//!
//! Everything except step 1's `RuntimeCall` construction is generic
//! to any Sovereign-SDK chain. The Ligate-specific piece is just
//! "wrap a `bank::CallMessage` in `RuntimeCall::Bank`" using the
//! re-exported runtime call enum from `ligate-stf`.

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use ligate_client::submit::Submitter;
use ligate_rollup::MockRollupSpec;
use ligate_stf::runtime::RuntimeCall;
use sov_bank::{Amount, CallMessage as BankCall, Coins, TokenId};
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::transaction::{PriorityFeeBips, UnsignedTransaction};
use sov_modules_api::{CryptoSpec, Spec};
use thiserror::Error;

/// Concrete spec for transaction construction.
///
/// `MockRollupSpec<Native>` carries the same address shape
/// (`MultiAddressEvm`) and runtime composition as the production
/// chain. The DA flavour (Mock vs. Celestia) is a property of the
/// running node, not of the transaction; the chain hash that binds
/// the signature is identical across DA flavours per
/// `crates/stf/build.rs`. So the faucet can sign with this spec and
/// the chain accepts the tx whether it's actually running MockDA
/// (localnet) or Celestia (devnet).
type S = MockRollupSpec<Native>;
type ChainRuntime = ligate_stf::runtime::Runtime<S>;
type SovPrivateKey = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey;
type SovAddress = <S as Spec>::Address;

/// Default per-tx fee envelope (in nano-LGT). Generous so a faucet
/// drip never fails for fee reasons even if the chain's per-tx gas
/// burn drifts up. Operators can tune via env if needed.
const DEFAULT_MAX_FEE_NANO: u128 = 100_000_000; // 0.1 $LGT

#[derive(Debug, Error)]
pub enum SignerError {
    #[error("invalid recipient address: {0}")]
    InvalidAddress(String),
    #[error("invalid signer key: {0}")]
    InvalidSignerKey(String),
    #[error("chain submission failed: {0}")]
    SubmitFailed(String),
}

#[derive(Debug, Clone)]
pub struct DripReceipt {
    /// Transaction hash returned by the chain.
    pub tx_hash: String,
    /// Drip amount in nano-LGT.
    pub amount_nano: u128,
}

pub struct Signer {
    private_key: SovPrivateKey,
    submitter: Submitter,
    chain_hash: [u8; 32],
    chain_id: u64,
    lgt_token_id: TokenId,
    /// Local-counter nonce. Initialised from chain at startup, then
    /// monotonically incremented per drip. If the faucet restarts,
    /// re-fetch from chain (operator-side concern, not a signer
    /// invariant).
    nonce: AtomicU64,
}

// Manual Debug to keep the signing key out of any debug prints.
impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("private_key", &"<redacted>")
            .field("chain_id", &self.chain_id)
            .field("nonce", &self.nonce.load(Ordering::Relaxed))
            .finish()
    }
}

impl Signer {
    pub fn new(
        signing_key_hex: &str,
        chain_rpc: String,
        chain_id: u64,
        chain_hash: [u8; 32],
        lgt_token_id: TokenId,
        starting_nonce: u64,
    ) -> Result<Self, SignerError> {
        if signing_key_hex.len() != 64 {
            return Err(SignerError::InvalidSignerKey(format!(
                "expected 64 hex chars, got {}",
                signing_key_hex.len()
            )));
        }
        let key_bytes = hex::decode(signing_key_hex)
            .map_err(|e| SignerError::InvalidSignerKey(format!("hex decode: {e}")))?;
        let private_key = SovPrivateKey::try_from(key_bytes)
            .map_err(|e| SignerError::InvalidSignerKey(format!("key shape: {e:?}")))?;

        Ok(Self {
            private_key,
            submitter: Submitter::new_unchecked(&chain_rpc),
            chain_hash,
            chain_id,
            lgt_token_id,
            nonce: AtomicU64::new(starting_nonce),
        })
    }

    /// Sign and submit a `bank.transfer` of `amount_nano` from the
    /// signer's address to `recipient`. Returns the chain-issued tx
    /// hash once the chain has executed (success or failure).
    pub async fn drip(
        &self,
        recipient: &str,
        amount_nano: u128,
    ) -> Result<DripReceipt, SignerError> {
        // Parse the recipient lig1... bech32m address.
        let to: SovAddress = SovAddress::from_str(recipient)
            .map_err(|e| SignerError::InvalidAddress(format!("{recipient}: {e}")))?;

        // Build the runtime call. RuntimeCall<S> is the chain's
        // composed dispatch enum; we construct the bank-module
        // variant.
        let runtime_call: RuntimeCall<S> = RuntimeCall::Bank(BankCall::Transfer {
            to,
            coins: Coins {
                amount: Amount::from(amount_nano),
                token_id: self.lgt_token_id,
            },
        });

        // Reserve a nonce for this drip. Atomic so concurrent
        // requests get distinct nonces. If the chain rejects this
        // tx (e.g., insufficient balance), the nonce is "burned"
        // until the chain marks it used by a subsequent successful
        // tx.
        let nonce = self.nonce.fetch_add(1, Ordering::SeqCst);

        // Wrap in unsigned tx envelope.
        let unsigned = UnsignedTransaction::<ChainRuntime, S>::new(
            runtime_call,
            self.chain_id,
            PriorityFeeBips::ZERO,
            Amount::from(DEFAULT_MAX_FEE_NANO),
            UniquenessData::Nonce(nonce),
            None, // gas_limit: None = chain-default
        );

        // Sign. Binds to chain_hash so the signature only verifies
        // on this chain id.
        let signed = unsigned.sign(&self.private_key, &self.chain_hash);

        // Borsh-encode the signed `Transaction`. The chain's
        // `POST /v1/sequencer/txs` handler accepts the inner signed tx
        // bytes directly and wraps them in `AuthenticatorInput::Standard`
        // server-side (see `sov-sequencer::rest_api::axum_accept_tx`).
        // Pre-wrapping here would double-wrap and the chain would
        // reject with "Cannot decompress Edwards point" (chain #245).
        let signed_bytes = borsh::to_vec(&signed)
            .map_err(|e| SignerError::SubmitFailed(format!("encoding signed tx: {e}")))?;

        // Submit.
        let tx_hash = self
            .submitter
            .submit_raw_tx(signed_bytes, /* wait */ true)
            .await
            .map_err(|e| SignerError::SubmitFailed(format!("submit: {e:#}")))?;

        Ok(DripReceipt {
            tx_hash: tx_hash.to_string(),
            amount_nano,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zeroed-out token id for unit tests. The signer doesn't actually
    /// touch the token id during construction, so the value is
    /// arbitrary; we just need _some_ `TokenId` to plug in.
    fn zero_token_id() -> TokenId {
        TokenId::from([0u8; 32])
    }

    #[test]
    fn rejects_too_short_key() {
        let err = Signer::new(
            "abcd",
            "http://localhost:12346".into(),
            1,
            [0u8; 32],
            zero_token_id(),
            0,
        )
        .unwrap_err();
        assert!(matches!(err, SignerError::InvalidSignerKey(_)));
    }

    #[test]
    fn rejects_non_hex_key() {
        let err = Signer::new(
            &"z".repeat(64),
            "http://localhost:12346".into(),
            1,
            [0u8; 32],
            zero_token_id(),
            0,
        )
        .unwrap_err();
        assert!(matches!(err, SignerError::InvalidSignerKey(_)));
    }

    #[test]
    fn accepts_valid_key() {
        let key = "00".repeat(32);
        let _ = Signer::new(
            &key,
            "http://localhost:12346".into(),
            1,
            [0u8; 32],
            zero_token_id(),
            0,
        )
        .unwrap();
    }
}
