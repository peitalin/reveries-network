# LLM Proxy

This MITM (Man-in-the-Middle) proxy intercepts API calls made by Python LLM services to multiple API providers (Anthropic, OpenAI, Google Gemini, etc.) and tracks token usage for each provider.

## How It Works

The proxy uses the Hudsucker library to serve as a MITM HTTP/S proxy that:

1. Intercepts outgoing API calls from the Python server to different LLM providers
2. Decrypts HTTPS traffic to inspect request and response contents
3. Tracks token usage per provider with detailed statistics
4. Logs complete request/response data for analysis
5. Works with any HTTP/HTTPS-based LLM API

## Key Features

- **MITM Capability**: Decrypts HTTPS traffic for inspection (requires CA certificate trust)
- **Multi-Provider Support**: Works with Anthropic, OpenAI, Google, and others
- **Token Usage Tracking**: Records and aggregates token consumption per provider
- **JSON Response Parsing**: Extracts structured data from API responses
- **Fallback Regex Extraction**: Can extract token data even from non-standard responses
- **Detailed Logging**: Comprehensive logs with timestamps and formatted responses

## Supported Providers

Currently configured for:

- Anthropic (Claude models)
- OpenAI (GPT models)
- Google (Gemini models)

Additional providers can be added by updating the provider mapping.

## Running with Docker

The easiest way to run the proxy along with LLM services is using the provided Docker setup:

```bash
docker-compose -f llm-services-compose.yml up -d
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

## Accessing Logs and Metrics

- Token usage is logged to the console in real-time
- Detailed logs with full request/response data are saved to the `./logs` directory
- Each log file is named with the provider and timestamp (e.g., `anthropic-2023-05-01_12-34-56.log`)

## Usage in Rust Applications

The `call_llm_api` function in `node/runtime/src/llm/metrics.rs` doesn't need any modifications.
It will continue to call the Python server at `http://localhost:8000/`, which in turn routes its
API requests through the proxy.

## Manual Setup

If you need to run the services separately:

```bash
# Start the proxy
cargo run -p llm-proxy

# In another terminal, set environment variables and start the Python server
export HTTP_PROXY=http://localhost:8080
export HTTPS_PROXY=http://localhost:8080
export NO_PROXY=localhost,127.0.0.1
cd agents/python/execute_with_memories
python main.py
```

## Technical Details

Under the hood, this proxy uses the [hudsucker](https://github.com/omjadas/hudsucker) library to:
1. Intercept and decrypt HTTPS traffic (MITM)
2. Inspect request and response bodies
3. Extract token usage information from JSON responses
4. Track and log usage statistics

The MITM approach allows the proxy to:
- See the actual content of encrypted traffic
- Extract token usage metrics from API responses
- Provide accurate usage statistics for all providers