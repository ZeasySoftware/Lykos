# syntax=docker/dockerfile:1

# --- Web-Client --------------------------------------------------------------
FROM node:24-slim AS web
WORKDIR /src/client
COPY client/package.json client/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY client/ ./
RUN npm run build

# --- Server ------------------------------------------------------------------
FROM rust:1.99-slim-trixie AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p lykos-server \
    && cp target/release/lykos-server /usr/local/bin/lykos-server

# --- Laufzeit ----------------------------------------------------------------
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /app lykos \
    && mkdir -p /app/data/files \
    && chown -R lykos:lykos /app/data
WORKDIR /app
COPY --from=server /usr/local/bin/lykos-server /usr/local/bin/lykos-server
COPY --from=web /src/client/dist /app/web
ENV LYKOS_BIND=0.0.0.0:8080 \
    LYKOS_WEB_DIR=/app/web \
    LYKOS_STORAGE_DIR=/app/data/files
USER lykos
EXPOSE 8080
CMD ["lykos-server"]
