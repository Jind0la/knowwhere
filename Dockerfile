ARG FEATURES=postgres-storage,summarizer
# To customize features at build time:
#   docker build --build-arg FEATURES=postgres-storage -t knowwhere-server:postgres .
#   docker compose build --build-arg FEATURES=postgres-storage,summarizer

FROM rust:1.86 AS builder

# Always install PostgreSQL client — needed for sqlx to compile against libpq
RUN apt-get update && apt-get install -y libpq-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release 2>/dev/null || true
ENV DATABASE_URL=postgresql://postgres:kw@localhost:5432/kw
ENV SQLX_OFFLINE=true
COPY .sqlx .sqlx
COPY src/ src/
COPY frontend/ frontend/
COPY benchmarks/ benchmarks/
RUN cargo build --release --features \"$FEATURES\"

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*
# Add iproute2 for 'ip' command (used by some network detection in Rust crates)
# curl is included for health checks and debugging
RUN apt-get update && apt-get install -y --no-install-recommends curl iproute2 && \
    rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/knowwhere-server /usr/local/bin/
COPY --from=builder /app/target/release/longmemeval_canary /usr/local/bin/
COPY --from=builder /app/target/release/longmemeval_qa_eval /usr/local/bin/
COPY --from=builder /app/target/release/longmemeval_retrieval_eval /usr/local/bin/
COPY --from=builder /app/frontend /app/frontend
COPY --from=builder /app/benchmarks/hf/fixtures /app/benchmarks/hf/fixtures
COPY migrations /app/migrations
COPY scripts/benchmark.sh /app/scripts/benchmark.sh
RUN chmod +x /app/scripts/benchmark.sh
WORKDIR /app
ENV RUST_LOG=info
# Ollama URL - inside Docker Compose network, use the ollama service name
ENV OLLAMA_API_URL=http://ollama:11434
ENV OLLAMA_MODEL=snowflake-arctic-embed2
ENV OLLAMA_VLM_MODEL=llama3.2
ENV KNOWWHERE_PORT=3737
EXPOSE 3737
CMD ["knowwhere-server"]
