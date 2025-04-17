import requests
import json
import asyncio
import subprocess
import os
import sys
import threading
from fastapi import FastAPI, HTTPException, Depends, BackgroundTasks, status
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel, Field
import uvicorn
import logging
import atexit
import signal
from typing import Dict, Any, List, Optional
from dotenv import load_dotenv

# MCP imports
try:
    from mcp.client import ClientSession, ManagedServerParameters
    from mcp.client.managed import managed_server
    MCP_AVAILABLE = True
except ImportError:
    print("MCP libraries not available. Tool capabilities will be disabled.")
    print("To enable MCP tools: pip install mcp-python-sdk httpx")
    MCP_AVAILABLE = False

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s',
)
logger = logging.getLogger("llm-api-gateway")

# Print a clear message at startup
print("="*80)
print(f"PYTHON FASTAPI SERVER STARTING - PID: {os.getpid()}")
print("="*80)

# Handle shutdown gracefully
def on_exit():
    print("="*80)
    print(f"PYTHON FASTAPI SERVER SHUTTING DOWN - PID: {os.getpid()}")
    print("="*80)

atexit.register(on_exit)

# Register signal handlers
for sig in [signal.SIGINT, signal.SIGTERM]:
    signal.signal(sig, lambda signum, frame: sys.exit(0))

# Load environment variables
load_dotenv()

app = FastAPI(title="LLM API Gateway")

# Add CORS middleware
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# Global MCP client session
mcp_client_session = None
mcp_client_lock = threading.Lock()

class QueryRequest(BaseModel):
    api_key: str
    prompt: str
    context: str

class TokenCount(BaseModel):
    input_tokens: int
    output_tokens: int
    total_tokens: int

class QueryResponse(BaseModel):
    text: str
    tokens: TokenCount

class ToolCall(BaseModel):
    name: str
    arguments: Dict[str, Any]

class MCPToolCall(BaseModel):
    tool_calls: List[ToolCall] = []

def start_mcp_server():
    """Start the MCP weather server as a subprocess"""
    if not MCP_AVAILABLE:
        return None

    try:
        # Path to the weather MCP server script
        server_script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "weather_mcp.py")

        # Start the server process
        process = subprocess.Popen(
            [sys.executable, server_script],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=0,
        )

        logger.info(f"Started MCP weather server with PID: {process.pid}")
        return process
    except Exception as e:
        logger.error(f"Failed to start MCP server: {e}")
        return None

async def init_mcp_client():
    """Initialize the MCP client connection"""
    global mcp_client_session

    if not MCP_AVAILABLE:
        return None

    with mcp_client_lock:
        if mcp_client_session is not None:
            return mcp_client_session

        try:
            # Start the MCP server process
            server_process = start_mcp_server()
            if server_process is None:
                logger.error("Failed to start MCP server")
                return None

            # Configure the managed server
            server_params = ManagedServerParameters(
                stdin=server_process.stdin,
                stdout=server_process.stdout,
                stderr=server_process.stderr,
                process=server_process
            )

            # Connect to the server
            transport = await managed_server(server_params)
            stdio, write = transport

            # Create a client session
            session = ClientSession(stdio, write)
            await session.initialize()

            mcp_client_session = session
            logger.info("MCP client initialized successfully")
            return session
        except Exception as e:
            logger.error(f"Failed to initialize MCP client: {e}")
            return None

async def get_available_tools():
    """Get a list of available tools from the MCP server"""
    session = await init_mcp_client()
    if session is None:
        return []

    try:
        # List available tools
        response = await session.list_tools()
        return [
            {
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.inputSchema
                }
            }
            for tool in response.tools
        ]
    except Exception as e:
        logger.error(f"Error getting available tools: {e}")
        return []

async def call_mcp_tool(name: str, arguments: Dict[str, Any]) -> str:
    """Call a tool via MCP and return the result"""
    session = await init_mcp_client()
    if session is None:
        return "Tool capabilities are not available"

    try:
        result = await session.call_tool(name, arguments)
        return result.content
    except Exception as e:
        logger.error(f"Error calling tool {name}: {e}")
        return f"Error calling tool {name}: {str(e)}"

def count_anthropic_tokens(api_key: str, content: str) -> int:
    """Count tokens for Anthropic API using their counting endpoint"""
    try:
        response = requests.post(
            "https://api.anthropic.com/v1/messages/count_tokens",
            headers={
                "x-api-key": api_key,
                "anthropic-version": "2023-06-01",
                "content-type": "application/json"
            },
            json={"model": "claude-3-haiku-20240307", "content": content}
        )
        response.raise_for_status()
        return response.json().get("token_count", 0)
    except Exception as e:
        logger.error(f"Error counting Anthropic tokens: {e}")
        return 0

def estimate_deepseek_tokens(text: str) -> int:
    """Rough estimate of tokens for DeepSeek based on whitespace-splitting"""
    # This is a very rough approximation - actual tokenization is more complex
    return len(text.split())

def estimate_tokens_from_text(text: str) -> int:
    """Estimate token count for a response."""
    # Most models average 3-4 characters per token
    return max(1, int(len(text) / 3.5))

def test_anthropic_query(api_key, prompt, context):
    """
    Makes a request to Anthropic's Claude API

    Args:
        api_key (str): Anthropic API key
        prompt (str): The question or prompt to send to the model
        context (str): The system context or instructions

    Returns:
        tuple: (response_text, token_count)
    """
    logger.info(f"Making request to Anthropic API with prompt: '{prompt[:30]}...'")
    url = "https://api.anthropic.com/v1/messages"
    headers = {
        "x-api-key": api_key,
        "anthropic-version": "2023-06-01",
        "content-type": "application/json"
    }

    # Format the messages for Claude
    messages = [
        {
            "role": "user",
            "content": prompt
        }
    ]

    # First, count the tokens
    token_counts = count_anthropic_tokens(
        api_key=api_key,
        content=prompt
    )
    input_tokens = token_counts

    # Then make the actual request
    payload = {
        "model": "claude-3-haiku-20240307",
        "max_tokens": 1000,
        "messages": messages
    }

    response = requests.post(url, headers=headers, json=payload)

    if response.status_code != 200:
        logger.error(f"Anthropic API error: {response.text}")
        raise Exception(f"API error: {response.text}")

    response_data = response.json()
    text = "".join([content["text"] for content in response_data["content"]
                   if content["type"] == "text"])

    # Get output token count from response
    output_tokens = response_data.get("usage", {}).get("output_tokens", estimate_tokens_from_text(text))

    logger.info(f"Received response from Anthropic API: '{text[:30]}...'")
    logger.info(f"Token usage - Input: {input_tokens}, Output: {output_tokens}, Total: {input_tokens + output_tokens}")

    return text, {
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": input_tokens + output_tokens
    }

def test_deepseek_query(api_key, prompt, context):
    """
    Makes a request to DeepSeek's Chat API

    Args:
        api_key (str): DeepSeek API key
        prompt (str): The question or prompt to send to the model
        context (str): The system context or instructions

    Returns:
        tuple: (response_text, token_count)
    """
    logger.info(f"Making request to DeepSeek API with prompt: '{prompt[:30]}...'")
    url = "https://api.deepseek.com/v1/chat/completions"
    headers = {
        "Authorization": f"Bearer {api_key}",
        "Content-Type": "application/json"
    }

    # For DeepSeek, we'll have to estimate token counts since they don't have a dedicated endpoint
    system_tokens = estimate_deepseek_tokens(context)
    prompt_tokens = estimate_deepseek_tokens(prompt)
    input_tokens = system_tokens + prompt_tokens

    payload = {
        "model": "deepseek-chat",
        "messages": [
            {
                "role": "system",
                "content": context
            },
            {
                "role": "user",
                "content": prompt
            }
        ],
        "max_tokens": 1000,
        "temperature": 0.7
    }

    response = requests.post(url, headers=headers, json=payload)

    if response.status_code != 200:
        logger.error(f"DeepSeek API error: {response.text}")
        raise Exception(f"DeepSeek API error: {response.text}")

    response_data = response.json()
    text = response_data["choices"][0]["message"]["content"]

    # Get token counts from response if available, otherwise estimate
    output_tokens = response_data.get("usage", {}).get("completion_tokens", estimate_tokens_from_text(text))
    actual_input_tokens = response_data.get("usage", {}).get("prompt_tokens", input_tokens)
    total_tokens = response_data.get("usage", {}).get("total_tokens", actual_input_tokens + output_tokens)

    logger.info(f"Received response from DeepSeek API: '{text[:30]}...'")
    logger.info(f"Token usage - Input: {actual_input_tokens}, Output: {output_tokens}, Total: {total_tokens}")

    return text, {
        "input_tokens": actual_input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": total_tokens
    }

async def anthropic_query_with_tools(api_key, prompt, context):
    """
    Makes a request to Anthropic's Claude API with tool capabilities
    """
    url = "https://api.anthropic.com/v1/messages"
    headers = {
        "x-api-key": api_key,
        "anthropic-version": "2023-06-01",
        "content-type": "application/json"
    }

    # Get available tools
    tools = await get_available_tools()

    if not tools:
        # If no tools available, fall back to regular query
        return test_anthropic_query(api_key, prompt, context)

    # Add tool context to the system prompt
    enhanced_context = context + "\nYou have access to weather tools that can provide forecasts and alerts. You can use these tools when weather information is needed."

    # Format the messages for Claude
    messages = [
        {
            "role": "user",
            "content": prompt
        }
    ]

    # First, count the tokens
    token_counts = count_anthropic_tokens(
        api_key=api_key,
        content=prompt
    )
    input_tokens = token_counts

    payload = {
        "model": "claude-3-haiku-20240307",
        "max_tokens": 1000,
        "messages": messages,
        "tools": tools
    }

    response = requests.post(url, headers=headers, json=payload)

    if response.status_code != 200:
        logger.error(f"Anthropic API error: {response.text}")
        raise Exception(f"API error: {response.text}")

    response_data = response.json()
    output_tokens = response_data.get("usage", {}).get("output_tokens", 0)
    total_tokens = response_data.get("usage", {}).get("total_tokens", input_tokens + output_tokens)

    # Check for tool_use in the response
    message = response_data.get("content", [])
    found_tool_use = False

    for block in message:
        if block.get("type") == "tool_use":
            found_tool_use = True
            tool_name = block.get("name")
            tool_input = block.get("input", {})

            # Call the tool
            logger.info(f"Calling tool {tool_name} with input {tool_input}")
            tool_result = await call_mcp_tool(tool_name, tool_input)

            # Make a second request with the tool results
            second_messages = [
                {
                    "role": "user",
                    "content": prompt
                },
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool_use",
                            "name": tool_name,
                            "input": tool_input
                        }
                    ]
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": block.get("id", ""),
                            "content": tool_result
                        }
                    ]
                }
            ]

            # Count tokens for the second request
            second_token_counts = count_anthropic_tokens(
                api_key=api_key,
                content=prompt + tool_result  # Approximate the additional content
            )
            second_input_tokens = second_token_counts
            input_tokens += second_input_tokens - input_tokens  # Only count the additional tokens

            second_payload = {
                "model": "claude-3-haiku-20240307",
                "max_tokens": 1000,
                "messages": second_messages
            }

            second_response = requests.post(url, headers=headers, json=second_payload)

            if second_response.status_code != 200:
                logger.error(f"Anthropic API error in second request: {second_response.text}")
                raise Exception(f"API error in second request: {second_response.text}")

            second_data = second_response.json()
            text = "".join([content["text"] for content in second_data["content"]
                           if content["type"] == "text"])

            # Update token counts with the second response
            second_output_tokens = second_data.get("usage", {}).get("output_tokens", estimate_tokens_from_text(text))
            output_tokens += second_output_tokens
            total_tokens = input_tokens + output_tokens

            return text, {
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "total_tokens": total_tokens
            }

    # If no tool use was found, just return the text
    if not found_tool_use:
        text = "".join([content["text"] for content in response_data["content"]
                       if content["type"] == "text"])
        return text, {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "total_tokens": total_tokens
        }

async def deepseek_query_with_tools(api_key, prompt, context):
    """
    Makes a request to DeepSeek's Chat API with tool capabilities
    """
    # DeepSeek doesn't natively support tools in the same way Anthropic does
    # We'll use a workaround by first checking if the query might need weather tools

    # Let's first do a simple query to see if weather info is needed
    weather_keywords = ["weather", "temperature", "forecast", "rain", "snow", "storm",
                       "alerts", "climate", "sunny", "cloudy", "humidity"]

    needs_weather = any(keyword in prompt.lower() for keyword in weather_keywords)

    if not needs_weather:
        # If it doesn't seem like a weather query, use regular API
        return test_deepseek_query(api_key, prompt, context)

    # Get available tools
    tools = await get_available_tools()

    if not tools:
        # If no tools available, fall back to regular query
        return test_deepseek_query(api_key, prompt, context)

    # Track token counts across multiple requests
    input_tokens = 0
    output_tokens = 0

    # Add tool description to the context
    tool_desc = "You can use these weather tools:\n"
    for tool in tools:
        func_info = tool["function"]
        tool_desc += f"- {func_info['name']}: {func_info['description']}\n"

    # First, ask DeepSeek if it would use a tool and which one
    tool_selection_prompt = f"""
You have access to the following weather-related tools:
{tool_desc}

For this user query: "{prompt}", would you use a weather tool? If yes, specify:
1. Which tool you would use (get_forecast or get_alerts)
2. The exact parameters you would pass (e.g., state code for alerts, latitude/longitude for forecast)

Return your answer in JSON format like this:
{{
  "use_tool": true or false,
  "tool_name": "get_forecast" or "get_alerts",
  "parameters": {{ ... }}
}}
"""

    # Enhanced context with tool instructions
    enhanced_context = context + "\n" + tool_desc + "\nYour task is to decide if a weather tool should be used, and if so, which one."

    # Call DeepSeek to decide on tool use
    tool_decision, first_tokens = test_deepseek_query(api_key, tool_selection_prompt, enhanced_context)
    input_tokens += first_tokens["input_tokens"]
    output_tokens += first_tokens["output_tokens"]

    try:
        # Parse the response to extract tool decision
        decision = json.loads(tool_decision)

        if decision.get("use_tool", False):
            tool_name = decision.get("tool_name")
            parameters = decision.get("parameters", {})

            # Call the tool
            logger.info(f"Calling tool {tool_name} with parameters {parameters}")
            tool_result = await call_mcp_tool(tool_name, parameters)

            # Now ask DeepSeek to formulate a response with the tool result
            final_prompt = f"""
Based on the user query: "{prompt}"

I used the {tool_name} tool and got this result:
{tool_result}

Please provide a helpful response to the user's query using this information.
"""

            # Call DeepSeek with the tool results
            final_response, final_tokens = test_deepseek_query(api_key, final_prompt, context)
            input_tokens += final_tokens["input_tokens"]
            output_tokens += final_tokens["output_tokens"]

            return final_response, {
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "total_tokens": input_tokens + output_tokens
            }
        else:
            # No tool use needed, use regular query
            return test_deepseek_query(api_key, prompt, context)

    except json.JSONDecodeError:
        # Couldn't parse the response as JSON, use regular query
        logger.warning("Failed to parse tool decision as JSON, falling back to regular query")
        regular_response, regular_tokens = test_deepseek_query(api_key, prompt, context)

        # Add the tokens from the first request
        regular_tokens["input_tokens"] += input_tokens
        regular_tokens["output_tokens"] += output_tokens
        regular_tokens["total_tokens"] = regular_tokens["input_tokens"] + regular_tokens["output_tokens"]

        return regular_response, regular_tokens

@app.on_event("startup")
async def startup_event():
    port = int(os.getenv("LLM_API_PORT", "8000"))
    logger.info(f"FastAPI server started on port {port} (PID: {os.getpid()})")
    logger.info("Available endpoints: /health, /anthropic, /deepseek")

    # Initialize MCP client on startup
    client = await init_mcp_client()
    if client:
        tools = await get_available_tools()
        logger.info(f"MCP initialized with {len(tools)} tools available")
    else:
        logger.warning("MCP client failed to initialize. Tool capabilities disabled.")

@app.on_event("shutdown")
async def shutdown_event():
    logger.info(f"FastAPI server shutting down (PID: {os.getpid()})")

    # Clean up MCP client
    global mcp_client_session
    if mcp_client_session:
        try:
            await mcp_client_session.cleanup()
            logger.info("MCP client cleaned up")
        except Exception as e:
            logger.error(f"Error cleaning up MCP client: {e}")

@app.post("/anthropic", response_model=QueryResponse)
async def query_anthropic(request: QueryRequest):
    logger.info("Received request to /anthropic endpoint")
    try:
        if MCP_AVAILABLE:
            response, tokens = await anthropic_query_with_tools(
                request.api_key,
                request.prompt,
                request.context
            )
        else:
            response, tokens = test_anthropic_query(
                request.api_key,
                request.prompt,
                request.context
            )
        return {"text": response, "tokens": tokens}
    except Exception as e:
        logger.error(f"Error processing /anthropic request: {str(e)}")
        raise HTTPException(status_code=500, detail=str(e))

@app.post("/deepseek", response_model=QueryResponse)
async def query_deepseek(request: QueryRequest):
    logger.info("Received request to /deepseek endpoint")
    try:
        if MCP_AVAILABLE:
            response, tokens = await deepseek_query_with_tools(
                request.api_key,
                request.prompt,
                request.context
            )
        else:
            response, tokens = test_deepseek_query(
                request.api_key,
                request.prompt,
                request.context
            )
        return {"text": response, "tokens": tokens}
    except Exception as e:
        logger.error(f"Error processing /deepseek request: {str(e)}")
        raise HTTPException(status_code=500, detail=str(e))

@app.get("/health")
async def health_check():
    logger.info("Health check request received")

    # Check if MCP client is initialized
    mcp_status = "available" if mcp_client_session else "unavailable"

    return {
        "status": "ok",
        "pid": os.getpid(),
        "mcp_status": mcp_status
    }

@app.get("/tools")
async def list_tools():
    """List all available tools from the MCP server"""
    tools = await get_available_tools()
    return {"tools": tools}

if __name__ == "__main__":
    # Run the FastAPI server
    port = int(os.getenv("LLM_API_PORT", "8000"))
    print(f"Starting uvicorn server on port {port}")
    uvicorn.run(app, host="0.0.0.0", port=port, log_level="info")
