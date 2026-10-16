# syntax=docker/dockerfile:1

FROM rust:1.99.0-slim-trixie AS chef

ARG TARGETARCH
RUN test "$TARGETARCH" = amd64

# QuickJS and AWS-LC compile C code; these tools never enter the runtime image.
RUN apt-get update \
    && apt-get install --no-install-recommends -y build-essential cmake pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --version 0.1.78 --locked

WORKDIR /build
COPY rust-toolchain.toml ./
# Fail on toolchain drift instead of silently building with a different Rust.
RUN rustc --version | grep -F "rustc ${RUST_VERSION} "

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY modules ./modules
COPY xtask ./xtask
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /build/recipe.json ./recipe.json
# Keep dependency artifacts in a layer: GHA cache does not export cache mounts.
RUN cargo chef cook --release --locked --bin limen --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY modules ./modules
COPY xtask ./xtask
RUN cargo build --release --locked --bin limen

FROM debian:trixie-slim AS runtime

LABEL org.opencontainers.image.licenses="AGPL-3.0-only"

RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates libgcc-s1 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 limen \
    && useradd --uid 10001 --gid 10001 --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin limen

COPY --from=builder /build/target/release/limen /usr/local/bin/limen
COPY LICENSE /usr/share/doc/limen/LICENSE

WORKDIR /
USER 10001:10001
EXPOSE 8080
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/limen"]
CMD ["serve"]
