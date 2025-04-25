"""
MCP Client Module

Based on the MCP Client Quickstart: https://modelcontextprotocol.io/quickstart/client
Manages connection to an MCP server (like weather_mcp.py) via stdio.
"""

import asyncio
import logging
from typing import Optional, Dict, Any
from contextlib import AsyncExitStack

# Import Request for the dependency getter
from fastapi import Request

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

logger = logging.getLogger("llm-api-gateway")

class MCPClient:
    def __init__(self):
        self.session: Optional[ClientSession] = None
        self.exit_stack = AsyncExitStack()
        self._is_connected = False

    async def connect_to_server(self, server_script_path: str):
        """Connect to an MCP server via stdio.

        Args:
            server_script_path: Absolute path to the server script (.py).
        """
        if self._is_connected:
            logger.info("MCPClient is already connected.")
            return

        logger.info(f"Attempting to connect to MCP server: {server_script_path}")
        if not server_script_path.endswith('.py'):
            raise ValueError("Server script must be a .py file for stdio connection")

        server_params = StdioServerParameters(
            command="python", # Assuming python environment has necessary packages
            args=[server_script_path],
            env=None # Inherit environment
        )

        try:
            # Enter the stdio_client context first
            stdio_transport = await self.exit_stack.enter_async_context(stdio_client(server_params))
            self.stdio, self.write = stdio_transport
            logger.info("MCP stdio transport established.")

            # Now enter the ClientSession context
            self.session = await self.exit_stack.enter_async_context(ClientSession(self.stdio, self.write))
            logger.info("MCP ClientSession established.")

            await self.session.initialize()
            logger.info("MCP Session initialized.")

            # List available tools upon connection
            response = await self.session.list_tools()
            tools = response.tools
            logger.info(f"Connected to MCP server with tools: {[tool.name for tool in tools]}")
            self._is_connected = True

        except Exception as e:
            logger.exception(f"Failed to connect or initialize MCP server {server_script_path}: {e}")
            # Ensure cleanup if connection fails midway
            await self.cleanup()
            raise

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
            # Assuming the result content is the string we need
            if isinstance(result.content, str):
                 return result.content
            else:
                 # Convert non-string results (like JSON) to string representation
                 return str(result.content)
        except Exception as e:
            logger.exception(f"MCP tool call '{tool_name}' failed: {e}")
            raise # Re-raise the exception to be handled by the caller

    async def cleanup(self):
        """Clean up resources and close the connection."""
        logger.info("Cleaning up MCPClient connection.")
        await self.exit_stack.aclose()
        self.session = None
        self._is_connected = False
        logger.info("MCPClient cleanup complete.")

# Dependency function to get the MCP client (moved from main.py)
def get_mcp_client(request: Request) -> Optional[MCPClient]:
    """FastAPI dependency function to get the MCP client instance from app state."""
    # Check if mcp_client exists in state and return it
    if hasattr(request.app.state, 'mcp_client'):
        return request.app.state.mcp_client
    return None