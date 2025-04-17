# LLM API Gateway

This is a simple FastAPI server that provides endpoints for querying different LLM APIs, including Anthropic's Claude and DeepSeek.

## Setup

1. Install the required Python packages:

```bash
pip install fastapi uvicorn requests pydantic python-dotenv
```

2. Create a `.env` file with your API keys (optional):

```
ANTHROPIC_API_KEY=your_anthropic_api_key
DEEPSEEK_API_KEY=your_deepseek_api_key
LLM_API_PORT=8000
```

## Running the Server

Start the FastAPI server with:

```bash
python test.py
```

By default, the server runs on port 8000. You can change this by setting the `LLM_API_PORT` environment variable.

## API Endpoints

### Health Check

```
GET /health
```

Returns `{"status": "ok"}` if the server is running.

### Query Anthropic Claude

```
POST /anthropic
```

Request body:
```json
{
  "api_key": "your_anthropic_api_key",
  "prompt": "What's the capital of France?",
  "context": "You are a helpful assistant."
}
```

### Query DeepSeek

```
POST /deepseek
```

Request body:
```json
{
  "api_key": "your_deepseek_api_key",
  "prompt": "What's the capital of France?",
  "context": "You are a helpful assistant."
}
```

## Using from Rust

The Rust code in `node/p2p-network/src/node_client/memories.rs` connects to these endpoints to query the LLMs. Make sure the FastAPI server is running before executing code that calls these endpoints.