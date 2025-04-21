import httpx
import logging
import sys
import asyncio
from typing import AsyncGenerator
from fastapi import HTTPException

# Get the logger instance (assuming configuration happens in main.py)
logger = logging.getLogger("llm-api-gateway")

async def call_anthropic(api_key: str, prompt: str, context: str, stream: bool = False):
    """
    Makes a request to Anthropic's Claude API, supporting streaming.
    Uses client.stream() for the streaming case.
    """
    logger.info(f"Making request to Anthropic API (stream={stream}) with prompt: '{prompt[:30]}...'")
    url = "https://api.anthropic.com/v1/messages"
    headers = {
        "x-api-key": api_key,
        "anthropic-version": "2023-06-01",
        "content-type": "application/json"
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

    if stream:
        payload["stream"] = True

    async def stream_generator() -> AsyncGenerator[bytes, None]:
        chunk_logger = logging.getLogger("llm-api-gateway") # Use the same logger name
        try:
            async with httpx.AsyncClient(timeout=None) as client:
                async with client.stream('POST', url, headers=headers, json=payload) as response:
                    response.raise_for_status()
                    async for chunk in response.aiter_bytes():
                        try:
                            log_message = f"Received chunk: {chunk.decode('utf-8')}"
                            chunk_logger.info(log_message)
                        except UnicodeDecodeError:
                            log_message = f"Received chunk (non-utf8): {chunk!r}"
                            chunk_logger.info(log_message)
                        # yield chunk # Yield original chunk
                        # Yield the log message formatted for SSE - OR keep yielding raw bytes?
                        # Let's stick to yielding raw bytes for StreamingResponse
                        yield chunk
        except httpx.HTTPStatusError as e:
            error_detail = f"Anthropic API Error: {await e.response.aread()}".encode('utf-8')
            logger.error(f"Anthropic API HTTP error (streaming): {e.response.status_code} - {error_detail.decode()}")
            # Yield an error message? Difficult with StreamingResponse handling raw bytes.
            # Best to let the exception propagate to the endpoint handler.
            raise # Re-raise the exception
        except Exception as e:
            error_detail = f"Internal stream error: {e}".encode('utf-8')
            logger.error(f"Generic error during Anthropic stream: {e}")
            raise # Re-raise the exception

    if stream:
        return stream_generator()
    else:
        async with httpx.AsyncClient(timeout=None) as client:
            try:
                response = await client.post(url, headers=headers, json=payload)
                response.raise_for_status()
                response_data = response.json()
                logger.info(f"Anthropic response body: {response_data}")
                text = "".join([content["text"] for content in response_data["content"]
                               if content["type"] == "text"])
                logger.info(f"Received response from Anthropic API: '{text[:30]}...'")
                return text
            except httpx.HTTPStatusError as e:
                error_detail = f"Anthropic API Error: {await e.response.aread()}".encode('utf-8')
                logger.error(f"Anthropic API HTTP error (non-streaming): {e.response.status_code} - {error_detail.decode()}")
                raise HTTPException(status_code=e.response.status_code, detail=error_detail.decode())
            except Exception as e:
                 error_detail = f"Internal Error during non-streaming request: {e}".encode('utf-8')
                 logger.error(f"Error during Anthropic non-streaming request: {e}")
                 raise HTTPException(status_code=500, detail=error_detail.decode())

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