# Backend image for TaskTips Cloud: API server, background worker and the
# one-shot migration share this image; the container command selects the
# process (tasktips-api [migrate|admin ...] or tasktips-worker).
#
# Published to Docker Hub as ke4nec/tasktips-cloud and consumed via
# ${TASKTIPS_BACKEND_IMAGE} in deploy/compose.yaml (local `build` still works
# as a fallback). It never bundles secrets: deploy/.env, deploy/secrets and
# key material stay excluded via .dockerignore.
FROM rust:1.97.1-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY migrations ./migrations
COPY contracts ./contracts
# Cache mounts keep the cargo registry/git checkouts and incremental build
# artifacts warm across builds; the CI workflows persist them through the
# BuildKit cache backend (type=gha,mode=max). Binaries are copied out because
# the target mount is discarded when the step finishes.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --locked --release --package tasktips-api --package tasktips-worker \
    && cp target/release/tasktips-api target/release/tasktips-worker /tmp/

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --no-install-recommends --yes ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /tmp/tasktips-api /tmp/tasktips-worker /usr/local/bin/

USER 65532:65532
EXPOSE 18080
CMD ["tasktips-api"]
