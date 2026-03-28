FROM rust:latest AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p willow-relay

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/willow-relay /usr/local/bin/willow-relay

EXPOSE 9090 9091
ENTRYPOINT ["willow-relay"]
CMD ["--tcp-port", "9090", "--ws-port", "9091", "--identity", "/etc/willow/relay.key"]
