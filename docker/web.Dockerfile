FROM rust:latest AS builder
RUN rustup target add wasm32-unknown-unknown
RUN cargo install trunk
WORKDIR /build
COPY . .
RUN cd crates/web && trunk build --release

FROM nginx:alpine
COPY --from=builder /build/crates/web/dist/ /usr/share/nginx/html/
EXPOSE 80
