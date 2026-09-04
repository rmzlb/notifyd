# syntax=docker/dockerfile:1
# Images épinglées par digest : le même commit produit le même binaire sur
# chaque serveur qui le build (une instance notifyd par entreprise, toutes
# déployées depuis ce dépôt). Refresh des pins volontaire, avec le Cargo.lock.
# cargo-chef arrive précompilé : plus de `cargo install` de plusieurs minutes
# à chaque perte de cache Docker.
FROM lukemathwalker/cargo-chef:latest-rust-1.98.0-alpine@sha256:917b051d1fc8e234a3aad123378b5263c95fa5d8739439ee25aa789c2db97a90 AS chef
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

FROM alpine:3.22@sha256:14358309a308569c32bdc37e2e0e9694be33a9d99e68afb0f5ff33cc1f695dce AS runtime
RUN apk add --no-cache ca-certificates wget
WORKDIR /app
COPY --from=builder /app/target/release/notifyd /usr/local/bin/notifyd
COPY --from=builder /app/migrations ./migrations
USER nobody
EXPOSE 3400
ENV RUST_LOG=notifyd=info
ENTRYPOINT ["/usr/local/bin/notifyd"]
