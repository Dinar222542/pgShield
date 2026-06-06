# syntax=docker/dockerfile:1
FROM rust:slim-bookworm AS builder
WORKDIR /app
# redis is pure Rust, no C library dependencies
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY config config
RUN cargo build --release --package pgshield-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates curl nfs-common postgresql-client && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/pgshield-server /usr/local/bin/
COPY --from=builder /app/config /etc/pgshield/config
COPY static /etc/pgshield/static
WORKDIR /etc/pgshield
EXPOSE 8080
ENTRYPOINT ["pgshield-server", "--config", "/etc/pgshield/config/default.yaml", "--data-dir", "/data", "--seed"]
