"""
MCP Client Module

Based on the MCP Client Quickstart: https://modelcontextprotocol.io/quickstart/client
Manages connection to an MCP server (like weather_mcp.py) via stdio.
"""

import asyncio
import logging
from typing import Optional, Dict, Any
from contextlib import AsyncExitStack
from fastapi import Request

from mcp.client.session import ClientSession
from mcp.client.sse import sse_client

logger = logging.getLogger("python-llm-server")

class MCPClient:
    def __init__(self):
        self.session: Optional[ClientSession] = None
        self.exit_stack = AsyncExitStack()
        self._is_connected = False
        self._http_url = None

    async def connect_to_server(self, script_path: str):
        """
        Connect to an MCP server by launching a Python script subprocess.
        This method is deprecated - use connect_to_http_server instead.

        Args:
            script_path: Path to the MCP server script (e.g. weather_mcp.py)
        """
        if self._is_connected:
            logger.info("MCPClient is already connected.")
            return

        logger.info(f"Attempting to connect to MCP server via script: {script_path}")
        # This code is deprecated - kept for backward compatibility
        # Implementation removed as it will be replaced by HTTP connection
        logger.warning("The connect_to_server method is deprecated. Use connect_to_http_server instead.")
        raise NotImplementedError("Direct script connection is deprecated. Use connect_to_http_server instead.")

    async def connect_to_http_server(self, server_url: str, max_retries: int = 5, retry_delay: float = 2.0):
        """Connect to an MCP server via HTTP.

        Args:
            server_url: The URL to the weather-mcp HTTP server
            max_retries: Maximum number of connection attempts (default: 5)
            retry_delay: Delay between retries in seconds (default: 2.0)
        """
        if self._is_connected:
            logger.info("MCPClient is already connected.")
            return

        logger.info(f"Attempting to connect to MCP server at: {server_url}")

        # Store the URL for later use
        self._http_url = server_url

        retry_count = 0
        last_error = None

        while retry_count < max_retries:
            try:
                # Use SSE client connection for HTTP
                sse_transport = await self.exit_stack.enter_async_context(sse_client(server_url))
                self.session = await self.exit_stack.enter_async_context(ClientSession(*sse_transport))
                logger.info("MCP HTTP SSE transport established.")

                await self.session.initialize()
                logger.info("MCP Session initialized.")

                # List available tools upon connection
                response = await self.session.list_tools()
                tools = response.tools
                logger.info(f"Connected to MCP server with tools: {[tool.name for tool in tools]}")
                self._is_connected = True
                return

            except Exception as e:
                last_error = e
                retry_count += 1
                logger.warning(f"Connection attempt {retry_count}/{max_retries} failed: {e}")

                if retry_count < max_retries:
                    logger.info(f"Retrying in {retry_delay} seconds...")
                    await asyncio.sleep(retry_delay)
                    # Increase backoff for subsequent retries
                    retry_delay = min(retry_delay * 1.5, 10)

        # If we get here, all retries failed
        logger.error(f"Failed to connect to MCP server at {server_url} after {max_retries} attempts: {last_error}")
        # Ensure cleanup if connection fails
        await self.cleanup()
        raise ConnectionError(f"Could not connect to MCP server after {max_retries} attempts: {last_error}")

    async def call_mcp_tool(self, tool_name: str, tool_args: Dict[str, Any]) -> str:
        """Call a tool on the connected MCP server.

        Args:
            tool_name: The name of the tool to call.
            tool_args: A dictionary of arguments for the tool.

        Returns:
            The content result from the tool as a string.

        Raises:
            RuntimeError: If the client is not connected.
            Exception: If the tool call fails.
        """
        if not self.session or not self._is_connected:
            raise RuntimeError("MCPClient is not connected to the server.")

        logger.info(f"Calling MCP tool '{tool_name}' with args: {tool_args}")
        try:
            result = await self.session.call_tool(tool_name, tool_args)
            logger.info(f"MCP tool '{tool_name}' executed successfully.")

            # Convert the result content directly to string
            tool_result_str = str(result.content)
            logger.debug(f"MCP tool result converted to string: {tool_result_str[:100]}...")
            return tool_result_str

        except Exception as e:
            logger.exception(f"MCP tool call '{tool_name}' failed: {e}")
            raise # Re-raise the exception to be handled by the caller

    async def cleanup(self):
        """Clean up resources and close the connection."""
        logger.info("Cleaning up MCPClient connection.")
        await self.exit_stack.aclose()
        self.session = None
        self._is_connected = False
        self._http_url = None
        logger.info("MCPClient cleanup complete.")

# Dependency function to get the MCP client (moved from main.py)
def get_mcp_client(request: Request) -> Optional[MCPClient]:
    """FastAPI dependency function to get the MCP client instance from app state."""
    # Check if mcp_client exists in state and return it
    if hasattr(request.app.state, 'mcp_client'):
        return request.app.state.mcp_client
    return None