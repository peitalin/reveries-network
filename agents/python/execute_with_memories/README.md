# LLM API Gateway

A simple FastAPI server that provides a unified interface to query multiple LLM providers (Anthropic Claude and DeepSeek) with optional weather tool capabilities.

## Installation

1. Clone this repository
2. Install dependencies:

```bash
pip install fastapi uvicorn requests pydantic python-dotenv
```

3. Optional: Install MCP SDK for tool capabilities:

```bash
pip install mcp-python-sdk httpx
```

4. Create a `.env` file in the same directory as the server:

```
ANTHROPIC_API_KEY=your_anthropic_api_key
DEEPSEEK_API_KEY=your_deepseek_api_key
LLM_API_PORT=8000
```

## Running the Server

Start the server with:

```bash
python main.py
```

The server runs on port 8000 by default or the port specified in the `LLM_API_PORT` environment variable.

## API Endpoints

### Query Anthropic Claude

```
POST /anthropic
```

Request body:
```json
{
  "api_key": "your_anthropic_api_key",
  "prompt": "What's the weather like in Miami?",
  "context": "You are a helpful assistant."
}
```

Response:
```json
{
  "text": "The response from Claude...",
  "tokens": {
    "input_tokens": 15,
    "output_tokens": 42,
    "total_tokens": 57
  }
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
  "prompt": "What's the weather like in Miami?",
  "context": "You are a helpful assistant."
}
```

Response: Same structure as Anthropic endpoint.

### Health Check

```
GET /health
```

Returns status information:
```json
{
  "status": "ok",
  "pid": 12345,
  "mcp_status": "available"
}
```

### List Available Tools

```
GET /tools
```

Returns list of available MCP tools (if MCP is available).

## Additional Features

- Automatic token counting for API requests
- Weather forecasting and alerts using MCP tools (when available)
- Support for both standard responses and tool-augmented responses