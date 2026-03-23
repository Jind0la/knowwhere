FROM rust:1.85 AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release 2>/dev/null || true
COPY src/ src/
COPY frontend/ frontend/
RUN cargo build --release --features postgres-storage

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/knowwhere-server /usr/local/bin/
COPY --from=builder /app/frontend /app/frontend
WORKDIR /app
ENV RUST_LOG=info
EXPOSE 3000
CMD ["knowwhere-server"]
