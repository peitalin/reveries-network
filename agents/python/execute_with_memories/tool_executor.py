"""
Tool Executor

This module handles the execution of tools requested by Anthropic's Claude models.
It processes tool_use requests, executes the appropriate tools using the MCPClient,
and prepares follow-up responses to send back to the model.
"""

import logging
import json
import asyncio
import os # Added os import
from typing import Dict, Any, List, Optional, Union, Tuple
import httpx
from fastapi import HTTPException

# Import MCPClient to use it for type hinting
from mcp_client import MCPClient

logger = logging.getLogger("llm-api-gateway")

# Define the set of known weather tools handled by MCPClient
KNOWN_WEATHER_TOOLS = {"get_forecast", "get_alerts"}

# The subprocess execution function is removed
# async def execute_weather_mcp_tool(...)

# TOOL_EXECUTORS dictionary is removed

async def process_tool_use_response(
    response_data: Dict[str, Any],
    mcp_client: Optional[MCPClient] # Accept MCPClient instance
) -> Tuple[Optional[str], Optional[str], str]:
    """
    Process a tool_use response from Anthropic API using the provided MCPClient.

    Args:
        response_data: The raw response data from Anthropic API.
        mcp_client: An instance of the connected MCPClient.

    Returns:
        A tuple of (tool_use_id, tool_name, tool_result).
        Returns (None, None, error_message) if processing fails.
    """
    tool_use_content = None
    tool_use_id = None
    tool_name = None
    tool_input = None

    # Find the tool_use content block
    for content in response_data.get("content", []):
        if content.get("type") == "tool_use":
            tool_use_content = content
            break

    if not tool_use_content:
        logger.error("Tool use response doesn't contain a tool_use content block")
        return None, None, "Error: No tool_use content block found"

    tool_use_id = tool_use_content.get("id")
    tool_name = tool_use_content.get("name")
    tool_input = tool_use_content.get("input", {})

    if not tool_use_id or not tool_name:
        logger.error(f"Missing tool_use_id or tool_name in response: {tool_use_content}")
        return None, None, "Error: Missing tool ID or name in response"

    logger.info(f"Processing tool use: {tool_name} (ID: {tool_use_id}) with input: {tool_input}")

    # Check if the tool is one handled by our MCPClient
    if tool_name in KNOWN_WEATHER_TOOLS:
        if not mcp_client:
            logger.error(f"MCPClient not available to execute tool: {tool_name}")
            return tool_use_id, tool_name, f"Error: Weather tool client not available"

        # Execute the tool using MCPClient
        try:
            tool_result = await mcp_client.call_mcp_tool(tool_name, tool_input)
            logger.info(f"Tool execution successful for {tool_name} via MCPClient")
            return tool_use_id, tool_name, tool_result
        except Exception as e:
            # Error already logged in call_mcp_tool
            logger.error(f"MCP tool call failed for {tool_name}")
            return tool_use_id, tool_name, f"Error executing {tool_name} via MCP: {str(e)}"
    else:
        # Tool is not supported by this executor
        logger.error(f"Tool '{tool_name}' is not supported by this executor.")
        return tool_use_id, tool_name, f"Error: Tool '{tool_name}' is not supported"

async def make_follow_up_request(
    original_response: Dict[str, Any],
    tool_use_id: str,
    tool_result: str,
    tools: Optional[List[Dict[str, Any]]] = None
) -> Dict[str, Any]:
    """
    Make a follow-up request to Anthropic API with the tool result.
    Reads ANTHROPIC_API_KEY from environment and adds x-api-key header if found.
    """
    logger.info(f"Making follow-up request with tool result for tool_use_id: {tool_use_id}")

    url = "https://api.anthropic.com/v1/messages"
    headers = {
        "anthropic-version": "2023-06-01",
        "content-type": "application/json",
        "anthropic-beta": "tools-2024-04-04"
    }

    # Attempt to add API key from environment
    api_key = os.getenv("ANTHROPIC_API_KEY")
    if api_key:
        logger.debug("Follow-up: Found ANTHROPIC_API_KEY in env, adding x-api-key header.")
        headers["x-api-key"] = api_key
    else:
        logger.warning("Follow-up: ANTHROPIC_API_KEY not found in environment. Request might fail if proxy doesn't inject it.")

    # Reconstruct the conversation history
    # Need to find the original user message(s) that led to the tool use
    # This information isn't directly in original_response in the current structure.
    # For now, we'll assume the immediate prior message in a hypothetical history was the user query.
    # A more robust solution would involve passing the message history through.

    # Simplified history reconstruction:
    messages = []
    # Placeholder: Assume the original prompt was the user message.
    # This needs improvement if multi-turn conversations are involved before tool use.
    # We retrieve it from the original_response which DOES contain it if call_anthropic populated it.
    # Let's check if original_response contains 'messages' field from the initial call.
    if "messages" in original_response:
         messages.extend(original_response["messages"]) # Add original message history
    else:
         # Fallback if initial messages aren't in the dict (should be fixed if this happens)
         logger.warning("Original messages not found in response_data for follow-up. Using placeholder.")
         # messages.append({"role": "user", "content": "[Placeholder: Original Prompt]"})
         pass # If no messages field, just continue with assistant/tool_result

    # Add the assistant response containing the tool_use request
    assistant_message = {
        "role": "assistant",
        "content": original_response.get("content", [])
    }
    # Filter out any previous tool_result messages if they somehow got included
    if isinstance(assistant_message["content"], list):
        assistant_message["content"] = [c for c in assistant_message["content"] if c.get("type") != "tool_result"]

    messages.append(assistant_message)

    # Add the tool result message
    tool_result_message = {
        "role": "user", # Tool results are sent with the user role
        "content": [
            {
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": tool_result
            }
        ]
    }
    messages.append(tool_result_message)

    # Prepare the payload
    payload = {
        "model": original_response.get("model", "claude-3-haiku-20240307"),
        "max_tokens": 4096,
        "messages": messages, # Use the reconstructed message history
    }

    # Include tools if they were in the original request
    if tools:
        payload["tools"] = tools
        payload["tool_choice"] = {"type": "auto"}

    logger.debug(f"Follow-up Request Payload: {json.dumps(payload, indent=2)}")

    # Make the request
    async with httpx.AsyncClient(timeout=None) as client:
        try:
            response = await client.post(url, headers=headers, json=payload)
            response.raise_for_status()
            response_data = response.json()
            logger.info(f"Follow-up response received from Anthropic API")
            logger.debug(f"Follow-up Response Body: {response_data}")

            # Extract text if it's not another tool use
            if response_data.get("stop_reason") != "tool_use":
                text = "".join([
                    content["text"]
                    for content in response_data.get("content", [])
                    if content.get("type") == "text"
                ])
                logger.info(f"Follow-up response text: '{text[:100]}...'")

            return response_data
        except httpx.HTTPStatusError as e:
            error_detail = await e.response.text()
            logger.error(f"HTTP Error making follow-up request: {e.response.status_code} - {error_detail}")
            raise HTTPException(status_code=e.response.status_code, detail=f"Anthropic follow-up error: {error_detail}")
        except Exception as e:
            logger.exception(f"Error making follow-up request: {e}")
            raise HTTPException(status_code=500, detail=f"Internal error during follow-up request: {str(e)}")