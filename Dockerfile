FROM rust:1.86 AS builder
ARG FEATURES=postgres-storage,voyage-provider,deepseek-summarizer,metrics
RUN apt-get update && apt-get install -y libpq-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release --no-default-features --features "$FEATURES" 2>/dev/null || true

ENV DATABASE_URL=postgresql://postgres:placeholder@localhost:5432/kw
ENV SQLX_OFFLINE=true
COPY .sqlx .sqlx
COPY src/ src/
COPY frontend/ frontend/
COPY benchmarks/ benchmarks/
RUN cargo build --release --no-default-features --features "$FEATURES"

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates curl iproute2 && \
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
ENV KNOWWHERE_EMBEDDING_PROVIDER=voyage
ENV KNOWWHERE_PORT=3737
EXPOSE 3737
CMD ["knowwhere-server"]