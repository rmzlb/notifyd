# syntax=docker/dockerfile:1
FROM rust:1-alpine AS chef
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static
RUN cargo install cargo-chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin notifyd

FROM alpine:3.21 AS runtime
RUN apk add --no-cache ca-certificates wget
WORKDIR /app
COPY --from=builder /app/target/release/notifyd /usr/local/bin/notifyd
COPY --from=builder /app/migrations ./migrations
USER nobody
EXPOSE 3400
ENV RUST_LOG=notifyd=info
ENTRYPOINT ["/usr/local/bin/notifyd"]
