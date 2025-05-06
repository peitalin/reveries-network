import httpx
import logging
import sys
import asyncio
import os
from typing import AsyncGenerator, Optional, List, Dict, Any, Union
from fastapi import HTTPException

# NEW: Import Anthropic SDK components
from anthropic import AsyncAnthropic
from anthropic import APIError, APITimeoutError
from anthropic.types import Message, TextBlock, ToolUseBlock
from anthropic.lib.streaming import AsyncMessageStream, MessageStreamEvent
from anthropic._types import Omit # Import Omit

logger = logging.getLogger("llm-api-gateway")

# Pre-defined tool definitions
WEATHER_TOOLS = [
    {
        "name": "get_forecast",
        "description": "Get weather forecast for a location based on latitude and longitude.",
        "input_schema": {
            "type": "object",
            "properties": {
                "latitude": {
                    "type": "number",
                    "description": "The latitude of the location"
                },
                "longitude": {
                    "type": "number",
                    "description": "The longitude of the location"
                }
            },
            "required": ["latitude", "longitude"]
        }
    },
    {
        "name": "get_alerts",
        "description": "Get active weather alerts for a US state.",
        "input_schema": {
            "type": "object",
            "properties": {
                "state": {
                    "type": "string",
                    "description": "Two-letter US state code (e.g. CA, NY)"
                }
            },
            "required": ["state"]
        }
    }
]

async def call_anthropic(
    prompt: str,
    context: Optional[str],
    stream: bool = False,
    tools: Optional[List[Dict[str, Any]]] = None
) -> Union[str, Dict[str, Any], AsyncGenerator[bytes, None]]:
    """
    Makes a request to Anthropic's Claude API using the Anthropic Python SDK,
    supporting streaming and tool use.
    Reads ANTHROPIC_API_KEY from environment automatically.
    """
    logger.info(f"Making request via Anthropic SDK (stream={stream}, tools={'yes' if tools else 'no'}) with prompt: '{prompt[:30]}...'")

    # Initialize the async client (SDK handles API key from env var ANTHROPIC_API_KEY)
    # Default timeout is 10 minutes, retries are handled by SDK
    try:
        # Initialize Anthropic with NO API key (it will read from env var ANTHROPIC_API_KEY which is set to a dummy variable)
        # This satisfies the SDK's initial check.
        # The proxy will inject the correct header via the http_client.
        client = AsyncAnthropic(
            # api_key="sk-user-supplied-api-key",
            ### API key provided, no API Key delegation
            api_key=None,
            ### No API key provided, API key will be injected by the proxy
        )
    except Exception as e:
        logger.error(f"Failed to initialize Anthropic client: {e}")
        raise HTTPException(status_code=500, detail=f"Anthropic client initialization error: {e}")

    messages = [{
        "role": "user",
        "content": prompt
    }]

    # Prepare common arguments for the API call
    api_args = {
        "model": "claude-3-haiku-20240307", # Or choose another model
        "max_tokens": 4096, # Adjust as needed
        "messages": messages,
    }

    if context:
        logger.debug("Adding system context to API call.")
        api_args["system"] = context
    else:
        logger.debug("No system context provided.")

    if tools:
        logger.debug("Adding tools to API call.")
        api_args["tools"] = tools
        # SDK defaults to auto tool choice if tools are provided

    if stream:
        api_args["stream"] = True
        async def stream_generator() -> AsyncGenerator[bytes, None]:
            chunk_logger = logging.getLogger("llm-api-gateway")
            try:
                # Use the async message stream context manager
                async with client.messages.stream(**api_args) as message_stream:
                    async for event in message_stream:
                        # Log the raw event type for debugging
                        # chunk_logger.debug(f"Stream Event Type: {type(event)}")

                        # Determine what data to yield based on event type
                        if isinstance(event, MessageStreamEvent) and hasattr(event, 'type'):
                             # Encode the event data (often a delta) as JSON bytes
                             # This matches the raw HTTP stream structure more closely
                             try:
                                 # Construct a JSON line similar to the HTTP SSE format
                                 sse_line = f"event: {event.type}\ndata: {event.model_dump_json()}\n\n"
                                 chunk_logger.debug(f"Yielding SSE line: {sse_line.strip()}")
                                 yield sse_line.encode('utf-8')
                             except Exception as encode_err:
                                 chunk_logger.error(f"Error encoding stream event {event.type}: {encode_err}")

            except APIError as e:
                error_detail = f"Anthropic SDK API Error (streaming): {e.status_code} - {e.body}".encode('utf-8')
                logger.error(f"Anthropic SDK API error (streaming): {e.status_code} - {e.body}")
                yield error_detail # Yield error detail as bytes
            except Exception as e:
                error_detail = f"Internal stream error: {e}".encode('utf-8')
                logger.error(f"Generic error during Anthropic SDK stream: {e}")
                yield error_detail # Yield error detail as bytes

        return stream_generator()
    else:
        # Non-streaming call
        api_args["stream"] = False
        try:
            response: Message = await client.messages.create(**api_args)
            logger.info(f"Anthropic SDK raw response model type: {type(response)}")
            # logger.info(f"Anthropic SDK response model content: {response}") # Can be verbose

            # Check for tool use
            if response.stop_reason == "tool_use":
                logger.info("Anthropic SDK response indicates tool use.")
                # Return the Pydantic model converted to a dictionary
                # Use model_dump for compatibility with newer Pydantic versions
                response_dict = response.model_dump(mode='json')
                logger.debug(f"Returning tool use response dict: {response_dict}")
                return response_dict
            else:
                # Extract text content
                text_parts = []
                if response.content:
                     for block in response.content:
                         if isinstance(block, TextBlock):
                             text_parts.append(block.text)

                full_text = "".join(text_parts)
                logger.info(f"Received standard text response via Anthropic SDK: '{full_text[:50]}...'")
                # Directly return the text string
                return full_text

        except APIError as e:
            logger.error(f"Anthropic SDK API error (non-streaming): {e.status_code} - {e.body}")
            # Try to extract detail, default if extraction fails
            detail = f"Anthropic SDK API Error: {e.body}" if e.body else f"Anthropic SDK API Error {e.status_code}"
            raise HTTPException(status_code=e.status_code or 500, detail=detail)
        except Exception as e:
            error_detail = f"Internal Error during non-streaming SDK request: {e}"
            logger.error(f"Error during Anthropic SDK non-streaming request: {e}")
            raise HTTPException(status_code=500, detail=error_detail)

def call_deepseek(api_key, prompt, context):
    """
    Makes a request to DeepSeek's Chat API (synchronous for simplicity).
    """
    logger.info(f"Making request to DeepSeek API with prompt: '{prompt[:30]}...'")
    url = "https://api.deepseek.com/v1/chat/completions"
    headers = {
        "Authorization": f"Bearer {api_key}",
        "Content-Type": "application/json"
    }

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
    try:
        # Note: Using synchronous requests here for simplicity as DeepSeek doesn't stream
        # A production app might use httpx.AsyncClient here too.
        import requests # Keep requests import here for sync call
        response = requests.post(url, headers=headers, json=payload)

        if response.status_code != 200:
            logger.error(f"DeepSeek API error: {response.status_code} {response.text}")
            raise HTTPException(status_code=response.status_code, detail=f"DeepSeek API error: {response.text}")

        response_data = response.json()
        text = response_data["choices"][0]["message"]["content"]

        logger.info(f"Received response from DeepSeek API: '{text[:30]}...'")
        return text
    except Exception as e:
        logger.error(f"Error during DeepSeek request: {e}")
        raise HTTPException(status_code=500, detail=f"Internal error during DeepSeek request: {e}")

async def test_anthropic_validation():
    """
    Test function to determine when the Anthropic SDK validates API keys.
    This can be called to verify whether validation happens at initialization time
    or at request time.
    """
    logger.info("Testing Anthropic SDK API key validation timing...")

    # Initialize with invalid key
    http_client = httpx.AsyncClient(timeout=60.0)
    client = AsyncAnthropic(
        api_key="sk-invalid-key",
        http_client=http_client,
    )
    logger.info("✓ Initialization with invalid key succeeded")

    # Attempt to make request (should fail if validation happens at request time)
    try:
        result = await client.messages.create(
            model="claude-3-haiku-20240307",
            max_tokens=10,
            messages=[{"role": "user", "content": "Hello"}]
        )
        logger.error("✗ Request with invalid key succeeded unexpectedly!")
        return "Unexpected success: API key not validated"
    except Exception as e:
        logger.info(f"✓ Request failed as expected: {e}")
        return f"Validation happens at request time: {e}"