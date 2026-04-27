FROM rust:latest AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p willow-replay

FROM rust:slim
RUN useradd -r -u 10001 -m -d /home/willow willow \
    && mkdir -p /etc/willow \
    && chown -R willow:willow /etc/willow
COPY --from=builder /build/target/release/willow-replay /usr/local/bin/willow-replay
COPY --chown=willow:willow docker/replay-entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

USER willow
ENTRYPOINT ["/entrypoint.sh"]
