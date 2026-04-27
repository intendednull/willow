FROM rust:latest AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p willow-relay

FROM rust:slim
RUN useradd -r -u 10001 -m -d /home/willow willow \
    && mkdir -p /etc/willow /shared \
    && chown -R willow:willow /etc/willow /shared
COPY --from=builder /build/target/release/willow-relay /usr/local/bin/willow-relay
COPY --chown=willow:willow docker/relay-entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

USER willow
EXPOSE 9090 9091
ENTRYPOINT ["/entrypoint.sh"]
