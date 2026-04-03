FROM rust:1-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY tarn ./tarn
COPY tarn-mcp ./tarn-mcp
COPY demo-server ./demo-server

RUN cargo build --release -p tarn -p tarn-mcp

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/tarn /usr/local/bin/tarn
COPY --from=builder /app/target/release/tarn-mcp /usr/local/bin/tarn-mcp

ENTRYPOINT ["tarn"]
CMD ["--help"]
