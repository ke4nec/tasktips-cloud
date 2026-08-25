FROM rust:1.97.1-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY migrations ./migrations
COPY contracts ./contracts
RUN cargo build --locked --release --package tasktips-api --package tasktips-worker

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --no-install-recommends --yes ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/tasktips-api /usr/local/bin/tasktips-api
COPY --from=builder /app/target/release/tasktips-worker /usr/local/bin/tasktips-worker

USER 65532:65532
EXPOSE 8080
CMD ["tasktips-api"]
