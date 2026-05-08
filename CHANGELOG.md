# Changelog

All notable changes to `ligate-faucet`. Pre-launch; everything sits
under `[Unreleased]` until the first tagged release alongside
`ligate-devnet-1` going live.

Format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- `.github/workflows/release.yml` — tagged-release workflow that
  cross-compiles `ligate-faucet` for the four target platforms
  operators run on (linux x86_64 / arm64, darwin arm64 / amd64),
  packages each as a `.tar.gz` with SHA-256 checksum, and attaches
  them to a GitHub Release with the `## [Unreleased]` section of this
  CHANGELOG as release notes. Triggered on `v*` tag pushes;
  `workflow_dispatch` runs the build matrix as a dry-run without
  publishing. Mirrors `ligate-chain`'s release workflow exactly so
  operators downloading both binaries get a uniform install
  experience. Drops "compile Rust on the GCP VM" from the deploy
  runbook (`ligate-chain/docs/development/public-devnet-deploy.md`)
  for the faucet leg.
- Real chain submission. Replaced the stubbed `signer.rs` with a
  full `bank.transfer` pipeline: builds `RuntimeCall::Bank(Transfer)`,
  wraps in `UnsignedTransaction`, signs against the chain's
  `CHAIN_HASH`, encodes via the chain's `RollupAuthenticator`, and
  submits via `ligate_client::submit::Submitter`. Closes the wiring
  gap from chain repo
  [#240](https://github.com/ligate-io/ligate-chain/issues/240) (which
  shipped the SDK pipeline) for the faucet's specific use case.
- New env vars required for chain identity:
    - `FAUCET_CHAIN_ID` (numeric, from `chain_state.json`)
    - `FAUCET_CHAIN_HASH` (64-char hex, from `/v1/rollup/info`)
    - `FAUCET_LGT_TOKEN_ID` (token id hex, from `bank.json`)
    - `FAUCET_STARTING_NONCE` (optional, default 0)
- New chain-side dependencies: `ligate-client` (with `submit`
  feature), `ligate-stf`, `ligate-rollup`, plus the SDK's `sov-bank`
  and `sov-modules-api`. All git-deps until upstream SDK lands on
  crates.io
  ([chain repo #235](https://github.com/ligate-io/ligate-chain/issues/235)).
- `constants.toml` mirroring chain repo's, required by Sovereign SDK
  macros to find compile-time constants (`GAS_DIMENSIONS` etc.).
  Copied verbatim; will drift if chain-side constants change. Future
  cleanup: chain repo could expose the constants via a published
  helper crate.

### Initial scaffold (earlier in same PR cycle)

- HTTP server (axum), per-address rate limiter (in-memory),
  env-driven config, Dockerfile, CI workflow, README. Tracking:
  [`ligate-chain#95`](https://github.com/ligate-io/ligate-chain/issues/95).
