# LLM Proxy

This proxy forks the Hudsucker library to serve as a MITM HTTP/S proxy that:

1. Intercepts outgoing API calls from the Python server to different LLM providers
2. Decrypts HTTPS traffic to inspect request and response contents
3. Tracks token usage per provider with detailed statistics
4. Logs complete request/response data for analysis
5. Works with any HTTP/HTTPS-based LLM API

This llm-proxy is run in a docker, alongside the p2p-node (that handles proxy re-encryption events, and spawns python llm agents inside docker) to track token usage.

## Supported Providers

Currently configured for Anthropic Claude

## Running with Docker

The easiest way to run the proxy along with LLM services is using the provided Docker setup:

```bash
docker-compose -f docker-compose-llm-proxy.yml up -d
```

This will:
1. Build and start the LLM proxy with MITM capabilities
2. Build and start the Python LLM server
3. Configure environment variables to route API calls through the proxy
4. Set up volume mounts for logs

## MITM Certificate Setup

For HTTPS interception to work properly, clients must trust the proxy's CA certificate. The proxy generates a self-signed certificate authority at runtime that can be exported and installed in client certificate stores.

When running the proxy for the first time, you'll need to:

1. Extract the CA certificate from the proxy container
2. Install it in your Python container's trusted certificate store
3. Set the appropriate environment variables for certificate validation


## Usage in Rust Applications

The `call_llm_api` function in `node/runtime/src/llm/mod.rs` doesn't need any modifications.
It will continue to call the Python server in docker-compose, which in turn routes its
API requests through the llm-proxy.

## Manual Setup

If you need to run the services separately:

```bash
# Start the proxy
cargo run -p llm-proxy

# In another terminal, set environment variables and start the Python server
export HTTP_PROXY=http://localhost:7666
export HTTPS_PROXY=http://localhost:7666
export NO_PROXY=localhost,127.0.0.1
cd agents/python/execute_with_memories
python main.py
```