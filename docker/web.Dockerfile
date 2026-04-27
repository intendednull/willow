FROM rust:latest AS builder
RUN rustup target add wasm32-unknown-unknown
RUN cargo install trunk
WORKDIR /build
COPY . .
RUN cd crates/web && trunk build --release

FROM nginxinc/nginx-unprivileged:alpine
COPY --from=builder --chown=nginx:nginx /build/crates/web/dist/ /usr/share/nginx/html/
RUN chmod 644 /usr/share/nginx/html/*

USER nginx
EXPOSE 8080
