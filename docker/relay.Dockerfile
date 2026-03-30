FROM rust:latest AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p willow-relay

FROM rust:slim
COPY --from=builder /build/target/release/willow-relay /usr/local/bin/willow-relay
COPY docker/relay-entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

EXPOSE 9090 9091
ENTRYPOINT ["/entrypoint.sh"]
