# rust:1.95-slim-bookworm pinned 2026-04-28; bump via `docker buildx imagetools inspect rust:1.95-slim-bookworm`
FROM rust:1.95-slim-bookworm@sha256:caaf9ca7acd474892186860307d6f28e51fdbc1a4eada459fcff81517cf46a36 AS builder
RUN rustup target add wasm32-unknown-unknown
RUN cargo install trunk
WORKDIR /build
COPY . .
RUN cd crates/web && trunk build --release

# nginxinc/nginx-unprivileged:1.27-alpine pinned 2026-04-28; bump via `docker buildx imagetools inspect nginxinc/nginx-unprivileged:1.27-alpine`
FROM nginxinc/nginx-unprivileged:1.27-alpine@sha256:65e3e85dbaed8ba248841d9d58a899b6197106c23cb0ff1a132b7bfe0547e4c0
COPY --from=builder --chown=nginx:nginx /build/crates/web/dist/ /usr/share/nginx/html/
RUN chmod 644 /usr/share/nginx/html/*

USER nginx
EXPOSE 8080
