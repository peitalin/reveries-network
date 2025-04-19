FROM rust:1.82 AS builder

WORKDIR /usr/src/llm-proxy
COPY . .

# Build the proxy
RUN cargo build --release -p llm-proxy

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Copy the compiled binary
COPY --from=builder /usr/src/llm-proxy/target/release/llm-proxy /usr/local/bin/

# Create directories for logs and certificates
RUN mkdir -p /logs && chmod 777 /logs
RUN mkdir -p /certs && chmod 777 /certs

# Expose proxy port
EXPOSE 8080

# Run the proxy
CMD ["llm-proxy"]