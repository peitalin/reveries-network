import httpx
import logging
import sys
import asyncio
from typing import AsyncGenerator, Optional, List, Dict, Any, Union
from fastapi import HTTPException

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
    api_key: str,
    prompt: str,
    context: str,
    stream: bool = False,
    tools: Optional[List[Dict[str, Any]]] = None
) -> Union[str, Dict[str, Any], AsyncGenerator[bytes, None]]:
    """
    Makes a request to Anthropic's Claude API, supporting streaming and tool use.
    Uses client.stream() for the streaming case.
    For non-streaming calls, returns the full response dict if tool use is detected,
    otherwise returns the extracted text.
    """
    logger.info(f"Making request to Anthropic API (stream={stream}, tools={'yes' if tools else 'no'}) with prompt: '{prompt[:30]}...'")
    url = "https://api.anthropic.com/v1/messages"
    headers = {
        "x-api-key": api_key,
        "anthropic-version": "2023-06-01",
        "content-type": "application/json",
        "anthropic-beta": "tools-2024-04-04"
    }

    messages = [
        {
            "role": "user",
            "content": prompt
        }
    ]

    payload = {
        "model": "claude-3-haiku-20240307",
        "max_tokens": 4096,
        "messages": messages,
    }

    if tools:
        payload["tools"] = tools
        payload["tool_choice"] = {"type": "auto"}

    if stream:
        payload["stream"] = True
        async def stream_generator() -> AsyncGenerator[bytes, None]:
            chunk_logger = logging.getLogger("llm-api-gateway")
            try:
                async with httpx.AsyncClient(timeout=None) as client:
                    async with client.stream('POST', url, headers=headers, json=payload) as response:
                        response.raise_for_status()
                        async for chunk in response.aiter_bytes():
                            try:
                                log_message = f"Received chunk: {chunk.decode('utf-8')}"
                                chunk_logger.debug(log_message)
                            except UnicodeDecodeError:
                                log_message = f"Received chunk (non-utf8): {chunk!r}"
                                chunk_logger.debug(log_message)
                            yield chunk
            except httpx.HTTPStatusError as e:
                try:
                    error_detail = f"Anthropic API Error: {await e.response.aread()}".encode('utf-8')
                    logger.error(f"Anthropic API HTTP error (streaming): {e.response.status_code} - {error_detail.decode()}")
                    yield error_detail
                except Exception as inner_e:
                     logger.error(f"Error reading error response body: {inner_e}")
                     raise e
            except Exception as e:
                error_detail = f"Internal stream error: {e}".encode('utf-8')
                logger.error(f"Generic error during Anthropic stream: {e}")
                yield error_detail

        return stream_generator()
    else:
        async with httpx.AsyncClient(timeout=None) as client:
            try:
                response = await client.post(url, headers=headers, json=payload)
                response.raise_for_status()
                response_data = response.json()
                logger.info(f"Anthropic raw response body: {response_data}")

                if response_data.get("stop_reason") == "tool_use":
                    logger.info("Anthropic response indicates tool use.")
                    return response_data
                else:
                    text = "".join([content["text"] for content in response_data.get("content", [])
                                   if content.get("type") == "text"])
                    logger.info(f"Received standard text response from Anthropic API: '{text[:30]}...'")
                    return text
            except httpx.HTTPStatusError as e:
                try:
                    error_detail_bytes = await e.response.aread()
                    error_detail = error_detail_bytes.decode('utf-8')
                    logger.error(f"Anthropic API HTTP error (non-streaming): {e.response.status_code} - {error_detail}")
                    raise HTTPException(status_code=e.response.status_code, detail=error_detail)
                except Exception as inner_e:
                     logger.error(f"Error reading error response body: {inner_e}")
                     raise HTTPException(status_code=e.response.status_code, detail="Anthropic API Error: Failed to read error details.") from inner_e
            except Exception as e:
                 error_detail = f"Internal Error during non-streaming request: {e}"
                 logger.error(f"Error during Anthropic non-streaming request: {e}")
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