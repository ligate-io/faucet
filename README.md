# `ligate-faucet`

Devnet faucet for Ligate Chain. Rate-limited HTTP service that drips
`$LGT` to addresses on `ligate-devnet-1` and onward.

> **Status: scaffold, not yet fully wired to chain submission.**
>
> The HTTP shape, rate limiter, config, and deploy story are all
> functional. The actual `bank.transfer` submission to the chain is
> stubbed pending a `transfer` helper landing in
> [`ligate-client`](https://github.com/ligate-io/ligate-chain/tree/main/crates/client-rs).
> Tracking: chain-side
> [#95](https://github.com/ligate-io/ligate-chain/issues/95).

## Why a separate faucet service

A public devnet means anyone can submit a transaction, but no one
has `$LGT` to pay the fees. A faucet drips `$LGT` to fresh addresses
so:

- Themisra evaluators can register a schema + submit attestations end-to-end
- Iris MCP testers can run the relayer against the public chain
- Mneme wallet QA can run integration tests
- Auditors / reviewers running published examples have executable accounts

Without a faucet, "devnet is public" reduces to "devnet is public if
you know someone at Ligate Labs."

The faucet runs as its own process (NOT part of `ligate-node`):

- Holds a hot Ed25519 signer key, address pre-funded at chain genesis
- Is **separate** from the bootstrap / treasury key so faucet drains
  don't touch treasury
- Rate-limits per recipient address (24h cooldown by default)

## HTTP API

### `POST /faucet`

```bash
curl -X POST https://faucet.ligate.io/faucet \
    -H 'Content-Type: application/json' \
    -d '{"address": "lig1xyz..."}'
```

**Success (200)**:

```json
{
  "address": "lig1xyz...",
  "tx_hash": "0x...",
  "amount_nano": 1000000000,
  "drip_amount_lgt": 1.0
}
```

**Rate-limited (429)**:

```json
{
  "error": "address rate-limited; retry in 86341 seconds",
  "retry_after_secs": 86341
}
```

**Bad address (400)**:

```json
{ "error": "expected lig1... bech32m, got celestia1...", "retry_after_secs": null }
```

### `GET /faucet/status`

Liveness + current-policy snapshot.

```json
{
  "drip_amount_nano": 1000000000,
  "drip_amount_lgt": 1.0,
  "rate_limit_secs": 86400,
  "addresses_dripped": 42
}
```

### `GET /health`

Always 200. Wire to k8s `livenessProbe` / load-balancer health checks.

## Configuration

All env vars (see [`.env.example`](.env.example)):

| Var | Required | Default | Notes |
|---|---|---|---|
| `FAUCET_BIND` | no | `0.0.0.0:8080` | HTTP bind address |
| `FAUCET_CHAIN_RPC` | **yes** | — | e.g. `https://rpc.ligate.io` |
| `FAUCET_SIGNER_KEY` | **yes** | — | 64-char hex Ed25519 private key |
| `FAUCET_DRIP_AMOUNT` | no | `1000000000` | nano-LGT per drip (1 LGT) |
| `FAUCET_RATE_LIMIT_SECS` | no | `86400` | cooldown per recipient address |
| `RUST_LOG` | no | `info,ligate_faucet=info` | tracing filter |

Generate a signer key with the chain repo's `ligate-genesis-tool`:

```bash
cd ligate-chain
cargo run -p ligate-genesis-tool -- keys generate \
    --roles faucet \
    --output ~/.ligate-keys/devnet-1

# The lig1... address from `~/.ligate-keys/devnet-1/faucet.address`
# must be pre-funded at chain genesis (substitute it into
# devnet-1/keys.toml under bank.json's address_and_balances).
#
# The 64-char hex from `~/.ligate-keys/devnet-1/faucet.key` is the
# value for FAUCET_SIGNER_KEY.
```

**Never commit a real `FAUCET_SIGNER_KEY`.** Inject via secret
manager (1Password CLI, GCP Secret Manager, AWS Secrets Manager).

## Deploying

### docker-compose

```yaml
services:
  faucet:
    image: ghcr.io/ligate-io/faucet:latest
    restart: unless-stopped
    environment:
      FAUCET_CHAIN_RPC: https://rpc.ligate.io
      FAUCET_SIGNER_KEY: ${FAUCET_SIGNER_KEY}  # from .env
      FAUCET_DRIP_AMOUNT: 1000000000
    ports:
      - "127.0.0.1:8080:8080"
```

Front with Caddy or Cloudflare Tunnel for public TLS at
`faucet.ligate.io`.

### From source

```bash
cargo run --release
```

Reads env from the shell. Recommended only for local dev; production
use the Docker image.

## Rate-limit design

In-memory per-address (Mutex<HashMap>). Trades durability for
simplicity:

- Faucet restart resets the window. Operators don't restart mid-day
  under normal conditions; if you do, anyone who already dripped can
  drip again immediately. Fine for v0.
- No per-IP limit. Adversaries can rotate IPs trivially (any cloud,
  any VPN); per-IP is theatre, per-address is the substantive
  defense.

For sustained-attack scenarios (sub-Sybil-resistant minting),
escalate via:

1. Persisting the rate-limit state to disk / Redis
2. Adding a small captcha or proof-of-work on the request
3. Reducing drip amount or extending the window

None of these are v0 work.

## Development

```bash
cargo build       # compile
cargo test        # unit tests
cargo run         # local server (requires .env)
cargo fmt         # format
cargo clippy --all-targets -- -D warnings
```

## Related

- Tracking issue: [`ligate-io/ligate-chain#95`](https://github.com/ligate-io/ligate-chain/issues/95)
- Chain SDK: [`ligate-io/ligate-chain/crates/client-rs`](https://github.com/ligate-io/ligate-chain/tree/main/crates/client-rs)
- Operator runbook: [`docs/development/public-devnet-deploy.md`](https://github.com/ligate-io/ligate-chain/blob/main/docs/development/public-devnet-deploy.md)
- Marketing-side faucet docs page: [`ligate-io/ligate-marketing#125`](https://github.com/ligate-io/ligate-marketing/issues/125)

## License

Apache-2.0 OR MIT, at your option. See [`LICENSE-APACHE`](LICENSE-APACHE)
and [`LICENSE-MIT`](LICENSE-MIT).
