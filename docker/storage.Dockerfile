FROM rust:latest AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p willow-storage

FROM rust:slim
COPY --from=builder /build/target/release/willow-storage /usr/local/bin/willow-storage
COPY docker/storage-entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
