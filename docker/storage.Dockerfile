FROM rust:latest AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p willow-storage

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/willow-storage /usr/local/bin/willow-storage

ENTRYPOINT ["willow-storage"]
CMD ["--identity-path", "/etc/willow/storage.key", "--db-path", "/var/lib/willow/storage.db"]
