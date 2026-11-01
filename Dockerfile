# syntax=docker/dockerfile:1.28
# Images épinglées par digest : le même commit produit le même binaire sur
# chaque serveur qui le build (une instance notifyd par entreprise, toutes
# déployées depuis ce dépôt). Refresh des pins volontaire, avec le Cargo.lock.
FROM rust:1.99.1-alpine3.24@sha256:7e9e2d068a107a2adafd19930011178d14a78326053cd4c3a9f7ad3fd049557d AS builder
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static git mold
WORKDIR /app

ENV CARGO_PROFILE_RELEASE_STRIP=symbols \
    RUSTFLAGS="-C link-arg=-fuse-ld=mold"

# .git reste dans le contexte (voir .dockerignore) : build.rs y lit le commit
# exposé par /v1/health, ce qui permet de comparer les instances déployées.
# Les caches BuildKit gardent le registre Cargo et `target/` hors des couches
# d'image (évite d'écrire 2,5 Go de couches par build et d'évincer les caches
# des autres services), et `.src-cache` restaure les mtime des fichiers
# inchangés après `git clone` pour ne recompiler que ce qui a changé.
COPY Cargo.toml Cargo.lock build.rs ./
COPY src src
COPY migrations migrations
COPY .git .git
RUN --mount=type=cache,id=notifyd-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=notifyd-cargo-target,target=/app/target,sharing=locked \
    rm -rf /app/target/release/incremental \
    && find Cargo.toml Cargo.lock build.rs src migrations -type f | while IFS= read -r f; do \
        if [ -f "/app/target/.src-cache/$f" ] && cmp -s "$f" "/app/target/.src-cache/$f"; then \
            touch -r "/app/target/.src-cache/$f" "$f"; \
        else \
            touch "$f" && mkdir -p "/app/target/.src-cache/$(dirname "$f")" && cp -p "$f" "/app/target/.src-cache/$f"; \
        fi; \
    done \
    && cargo build --release --locked --bin notifyd \
    && cp /app/target/release/notifyd /app/notifyd

FROM alpine:3.24.3@sha256:be2656061b536c1883c4034790e21d0a09c5aaaf51968b89368a3c0371ea2c9a AS runtime
# ca-certificates est requis : reqwest 0.13 (rustls-platform-verifier) lit le
# magasin système, et sans lui les clients HTTP ne démarrent pas.
RUN apk add --no-cache ca-certificates wget
WORKDIR /app
COPY --from=builder /app/notifyd /usr/local/bin/notifyd
COPY --from=builder /app/migrations ./migrations
LABEL org.opencontainers.image.source="https://github.com/rmzlb/notifyd" \
      org.opencontainers.image.licenses="MIT" \
      io.modelcontextprotocol.server.name="io.github.rmzlb/notifyd"
USER nobody
EXPOSE 3400
ENV RUST_LOG=notifyd=info
ENTRYPOINT ["/usr/local/bin/notifyd"]
