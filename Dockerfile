# syntax=docker/dockerfile:1
# Images épinglées par digest : le même commit produit le même binaire sur
# chaque serveur qui le build (une instance notifyd par entreprise, toutes
# déployées depuis ce dépôt). Refresh des pins volontaire, avec le Cargo.lock.
# cargo-chef arrive précompilé : plus de `cargo install` de plusieurs minutes
# à chaque perte de cache Docker.
FROM lukemathwalker/cargo-chef:latest-rust-1.99.1-alpine@sha256:2c27e67049f17e8b67d882a2cf2f5e1a1d4d71c206ec97f91edf278254688e95 AS chef
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static git
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
# .git reste dans le contexte (voir .dockerignore) : build.rs y lit le commit
# exposé par /v1/health, ce qui permet de comparer les instances déployées.
COPY . .
RUN cargo build --release --locked --bin notifyd

FROM alpine:3.24.3@sha256:be2656061b536c1883c4034790e21d0a09c5aaaf51968b89368a3c0371ea2c9a AS runtime
# ca-certificates est requis : reqwest 0.13 (rustls-platform-verifier) lit le
# magasin système, et sans lui les clients HTTP ne démarrent pas.
RUN apk add --no-cache ca-certificates wget
WORKDIR /app
COPY --from=builder /app/target/release/notifyd /usr/local/bin/notifyd
COPY --from=builder /app/migrations ./migrations
LABEL org.opencontainers.image.source="https://github.com/rmzlb/notifyd" \
      org.opencontainers.image.licenses="MIT" \
      io.modelcontextprotocol.server.name="io.github.rmzlb/notifyd"
USER nobody
EXPOSE 3400
ENV RUST_LOG=notifyd=info
ENTRYPOINT ["/usr/local/bin/notifyd"]
