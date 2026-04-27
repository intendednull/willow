FROM rust:latest AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p willow-storage

FROM rust:slim
RUN useradd -r -u 10001 -m -d /home/willow willow \
    && mkdir -p /etc/willow /var/lib/willow \
    && chown -R willow:willow /etc/willow /var/lib/willow
COPY --from=builder /build/target/release/willow-storage /usr/local/bin/willow-storage
COPY --chown=willow:willow docker/storage-entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

USER willow
ENTRYPOINT ["/entrypoint.sh"]
