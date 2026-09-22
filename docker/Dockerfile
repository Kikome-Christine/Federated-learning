# One image, all binaries (participant, scheduler, coordinator, flsim).
# Reproducibility: base images are pinned by DIGEST via `make pin`, which
# writes docker/digests.env; docker-compose passes them as build args.
ARG RUST_IMAGE=rust:1.90-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

FROM ${RUST_IMAGE} AS builder
WORKDIR /src
COPY Cargo.toml ./
COPY Cargo.loc[k] ./
COPY crates ./crates
# --locked makes the build fail if Cargo.lock is out of date (pinned deps).
RUN if [ -f Cargo.lock ]; then cargo build --release --locked --workspace; \
    else echo "WARNING: no Cargo.lock committed - build is NOT reproducible" >&2; \
         cargo build --release --workspace; fi

FROM ${RUNTIME_IMAGE} AS runtime
# iproute2 provides `tc` for netem (needs cap NET_ADMIN, granted in compose).
RUN apt-get update \
 && apt-get install -y --no-install-recommends iproute2 ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/participant /usr/local/bin/participant
COPY --from=builder /src/target/release/scheduler   /usr/local/bin/scheduler
COPY --from=builder /src/target/release/coordinator /usr/local/bin/coordinator
COPY --from=builder /src/target/release/flsim       /usr/local/bin/flsim
COPY config /app/config
WORKDIR /app
