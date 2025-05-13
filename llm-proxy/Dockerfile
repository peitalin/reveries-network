# Use the desired Rust base image
FROM rust:1.86 AS builder

# Install openssl needed for the script
RUN apt-get update && apt-get install -y --no-install-recommends openssl ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Set the working directory inside the builder
WORKDIR /usr/src/llm-proxy

# Copy the entire project context (including llm-proxy folder and generate_ca.sh)
COPY . .

# Build the proxy (ensure -p flag is used if it's part of a workspace)
RUN cargo build --release -p llm-proxy

# --- Final Stage ---
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Copy the compiled binary
COPY --from=builder /usr/src/llm-proxy/target/release/llm-proxy /usr/local/bin/

# Create directories (if needed at runtime, though not strictly required by proxy)
RUN mkdir -p /certs && chmod 777 /certs

# Expose proxy port
EXPOSE 7666
# Expose internal API port
EXPOSE 7070

# Run the proxy
CMD ["llm-proxy"]