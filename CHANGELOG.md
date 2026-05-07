# Changelog

All notable changes to `ligate-faucet`. Pre-launch; everything sits
under `[Unreleased]` until the first tagged release alongside
`ligate-devnet-1` going live.

Format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Initial scaffold: HTTP server (axum), per-address rate limiter
  (in-memory), env-driven config, signer module (stubbed pending
  `bank.transfer` helper landing in `ligate-client`), Dockerfile,
  CI workflow, README. Tracking: [`ligate-chain#95`](https://github.com/ligate-io/ligate-chain/issues/95).
