# Network Tests

This directory contains integration tests

## Prerequisites

Before running the tests, make sure you have installed:

1. Rust toolchain (cargo, etc.)
2. Python 3.7+ with the following packages:
   ```bash
   pip install fastapi uvicorn pydantic python-dotenv requests
   ```

## FastAPI Server

The memory tests require the Python FastAPI server to be installed and available. The server handles LLM API integrations for the memory functionality.

The tests will automatically:
1. Start the FastAPI server on port 8000
2. Verify the server is running properly
3. Clean up the server process when the test completes

If the Python server fails to start, check:
- That all required Python dependencies are installed
- That the file `agents/python/test.py` exists and is executable
- That port 8000 is available (not in use by another process)

## Running Tests

To run all tests:

```bash
cargo test
```

To run a specific test:

```bash
cargo test test_memory_reverie
```
```bash
cargo test --test proxy_api_test
```

To run a test with more detailed output:

```bash
cargo test test_memory_reverie -- --nocapture
```

## Troubleshooting

If tests fail with port-related errors, make sure to kill any zombie processes that might be holding the ports:

```bash
# Find processes on specific ports
lsof -i :8000  # Python FastAPI server
lsof -i :8001-8010  # RPC ports
lsof -i :9001-9010  # Listen ports

# Kill processes by PID
kill -9 [PID]
```

## API Keys

Some tests require API keys for external services (like Anthropic and DeepSeek). These should be provided via environment variables:

```bash
export ANTHROPIC_API_KEY=your_key_here
```

You can also create a `.env` file in the workspace root with these values.