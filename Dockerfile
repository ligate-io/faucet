# Multi-stage Dockerfile for `ligate-faucet`.
# Stage 1: build the static-ish binary in a Rust toolchain image.
# Stage 2: copy the binary into a slim runtime image.
#
# Targets only linux/amd64 + linux/arm64 (the operator-side audience).
# macOS / Windows operators run via `cargo run` directly.

FROM rust:1.93-bookworm AS builder

WORKDIR /build

# Cache deps separately from sources by copying only the manifest
# first. Cargo's incremental build then skips dep recompilation on
# source-only edits.
COPY Cargo.toml Cargo.lock* rust-toolchain.toml ./
RUN mkdir -p src && echo "fn main() {}" > src/main.rs && \
    cargo build --release --bin ligate-faucet && \
    rm -rf src

COPY src ./src
RUN cargo build --release --bin ligate-faucet && \
    strip target/release/ligate-faucet

# Stage 2: minimal runtime image. Just glibc + ca-certificates +
# the binary. ~50 MB final image.
FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Run as an unprivileged user. UID 1000 matches the most common
# host-side mapping; rebuild with --build-arg UID=NNN to match your
# operator's user.
ARG UID=1000
RUN useradd --system --uid ${UID} --shell /usr/sbin/nologin --create-home faucet
USER faucet
WORKDIR /home/faucet

COPY --from=builder --chown=faucet:faucet /build/target/release/ligate-faucet /usr/local/bin/ligate-faucet

# HTTP server port. Override at runtime with `-p HOST_PORT:8080` and
# `FAUCET_BIND=0.0.0.0:8080`.
EXPOSE 8080

# `FAUCET_SIGNER_KEY` MUST be injected at runtime via env or secret
# mount, never baked into the image.
ENTRYPOINT ["/usr/local/bin/ligate-faucet"]
