# rust:1.95-slim-bookworm pinned 2026-04-28; bump via `docker buildx imagetools inspect rust:1.95-slim-bookworm`
FROM rust:1.95-slim-bookworm@sha256:caaf9ca7acd474892186860307d6f28e51fdbc1a4eada459fcff81517cf46a36 AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p willow-storage

# rust:1.95-slim-bookworm pinned 2026-04-28; bump via `docker buildx imagetools inspect rust:1.95-slim-bookworm`
FROM rust:1.95-slim-bookworm@sha256:caaf9ca7acd474892186860307d6f28e51fdbc1a4eada459fcff81517cf46a36
RUN useradd -r -u 10001 -m -d /home/willow willow \
    && mkdir -p /etc/willow /var/lib/willow \
    && chown -R willow:willow /etc/willow /var/lib/willow
COPY --from=builder /build/target/release/willow-storage /usr/local/bin/willow-storage
COPY --chown=willow:willow docker/storage-entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

USER willow
ENTRYPOINT ["/entrypoint.sh"]
