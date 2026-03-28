FROM rust:latest AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p willow-replay

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/willow-replay /usr/local/bin/willow-replay

ENTRYPOINT ["willow-replay"]
CMD ["--identity-path", "/etc/willow/replay.key"]
