FROM rust:latest AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p willow-replay

FROM rust:slim
COPY --from=builder /build/target/release/willow-replay /usr/local/bin/willow-replay
COPY docker/replay-entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
