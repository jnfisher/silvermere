# syntax=docker/dockerfile:1
# Build from the repository root:  docker build -f docker/Dockerfile -t silvermere .
# Multi-arch: the build always runs on the build platform and cross-compiles to musl with
# cargo-zigbuild, so a linux/arm64 image needs no QEMU.
# RUST_VERSION is fed from rust-toolchain.toml by the release workflow; the default must match it.

ARG RUST_VERSION=1.99.0

FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-bookworm AS chef
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3-pip \
 && rm -rf /var/lib/apt/lists/*
RUN pip install --no-cache-dir --break-system-packages ziglang==0.13.0
RUN cargo install cargo-chef cargo-zigbuild --locked
RUN rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG TARGETARCH
# strip through rustc so the build host's binutils never sees a foreign-arch ELF
ENV CARGO_PROFILE_RELEASE_STRIP=symbols
COPY --from=planner /src/recipe.json recipe.json
# the [patch.crates-io] crates (T-111): cargo-chef's skeleton names them but holds no sources
COPY patches patches
RUN case "$TARGETARCH" in \
      amd64) echo x86_64-unknown-linux-musl > /target ;; \
      arm64) echo aarch64-unknown-linux-musl > /target ;; \
      *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac
RUN cargo chef cook --release --locked --zigbuild --target "$(cat /target)" --recipe-path recipe.json
COPY . .
RUN T="$(cat /target)" \
 && cargo zigbuild --release --locked --target "$T" -p silvermere --bin silvermere \
 && mkdir -p /out/data \
 && cp "target/$T/release/silvermere" /out/silvermere \
 && chown 65532:65532 /out/data

FROM gcr.io/distroless/static:nonroot
ARG VERSION=dev
ARG REVISION=unknown
LABEL org.opencontainers.image.source="https://github.com/jnfisher/silvermere" \
      org.opencontainers.image.title="silvermere" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"
COPY --from=builder /out/silvermere /silvermere
# A fresh named volume mounted on /data inherits this owner.
COPY --from=builder --chown=65532:65532 /out/data /data
# Relative defaults (sqlite://data/silvermere.db) land in /data.
WORKDIR /
# The binary defaults to a loopback admin bind; in a container the port is published instead.
# check_bind refuses a non-loopback admin bind without a token, so supply SILVERMERE_ADMIN_TOKENS
# (compose does, from .env).
ENV SILVERMERE_DATA_DIR=/data \
    SILVERMERE_SERVER__ADMIN_BIND=0.0.0.0:8080
VOLUME /data
EXPOSE 2323 8080
USER 65532:65532
HEALTHCHECK --interval=30s --timeout=3s --start-period=20s --retries=3 \
  CMD ["/silvermere","healthcheck"]
ENTRYPOINT ["/silvermere"]
CMD ["serve"]
