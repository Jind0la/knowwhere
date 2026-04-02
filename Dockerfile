# Build args for features
# Default: no features (in-memory only)
# To enable PostgreSQL + pgvector: docker build --build-arg FEATURES=postgres-storage -t knowwhere-server:postgres .
# IMPORTANT: When using postgres-storage feature, you must also run a PostgreSQL server
# with the pgvector extension. Use the docker-compose.yml which provides pgvector/pgvector:pg16.
# The knowwhere-server binary is a client — it connects to an external Postgres instance.
ARG FEATURES=

FROM rust:1.85 AS builder

# Install PostgreSQL client library only if postgres-storage feature is enabled
# (needed for sqlx to compile against libpq)
RUN case "$FEATURES" in *postgres*) apt-get update && apt-get install -y libpq-dev && rm -rf /var/lib/apt/lists/* ;; esac

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release 2>/dev/null || true
COPY src/ src/
COPY frontend/ frontend/
RUN if [ -n "${FEATURES}" ]; then cargo build --release --features ${FEATURES}; else cargo build --release; fi

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/knowwhere-server /usr/local/bin/
COPY --from=builder /app/frontend /app/frontend
WORKDIR /app
ENV RUST_LOG=info
EXPOSE 3000
CMD ["knowwhere-server"]
