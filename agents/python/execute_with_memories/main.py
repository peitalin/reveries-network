import asyncio
import dotenv
import httpx
import json
import logging
import os
import requests
import uvicorn
import sys
from contextlib import asynccontextmanager

from typing import Optional, AsyncGenerator
from fastapi import FastAPI, HTTPException, Request, status
from fastapi.exceptions import RequestValidationError
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import StreamingResponse, JSONResponse
from pydantic import BaseModel

# Import route modules
from routes import anthropic, deepseek, health
from mcp_client import MCPClient

# Load environment variables
dotenv.load_dotenv()

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s',
)
logger = logging.getLogger("python-llm-server")

# Determine absolute path for weather_mcp.py (adjust if needed based on container structure)
WEATHER_MCP_SCRIPT_PATH = os.path.abspath(os.path.join(os.path.dirname(__file__), "weather_mcp.py"))

async def validation_exception_handler(request: Request, exc: RequestValidationError):
    """
    Log detailed validation errors for 422 responses.
    """
    # Format the exception details for logging
    # exc.errors() provides more structure, but str(exc) is simpler for now
    exc_str = f'{exc}'.replace('\n', ' ').replace('   ', ' ')
    logger.error(f"Request Validation Error: {exc_str}")
    # Log details for easier debugging
    try:
        body = await request.json()
        logger.error(f"Request Body causing validation error: {json.dumps(body, indent=2)}")
    except Exception:
        logger.error("Request Body causing validation error could not be parsed as JSON.")

    # Return a JSON response similar to FastAPI's default, but with logged details
    return JSONResponse(
        content={"detail": exc.errors()}, # Use exc.errors() for structured detail
        status_code=status.HTTP_422_UNPROCESSABLE_ENTITY,
    )

@asynccontextmanager
async def lifespan(app: FastAPI):
    # Startup: Initialize and connect MCP Client
    logger.info("FastAPI startup: Initializing MCP Client...")
    mcp_client = MCPClient()
    try:
        await mcp_client.connect_to_server(WEATHER_MCP_SCRIPT_PATH)
        app.state.mcp_client = mcp_client
        logger.info("MCP Client connected successfully.")
    except Exception as e:
        logger.error(f"MCP Client connection failed during startup: {e}")
        app.state.mcp_client = None

    yield

    # Shutdown: Cleanup MCP Client
    logger.info("FastAPI shutdown: Cleaning up MCP Client...")
    if hasattr(app.state, 'mcp_client') and app.state.mcp_client:
        await app.state.mcp_client.cleanup()
        logger.info("MCP Client cleanup finished.")
    else:
        logger.info("No active MCP Client to clean up.")

# Create FastAPI app with lifespan management
app = FastAPI(title="LLM API Gateway", lifespan=lifespan)

# --- Register Exception Handler ---
app.add_exception_handler(RequestValidationError, validation_exception_handler)
# --- End Register Exception Handler ---

# Add CORS middleware
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# Include routers from route modules
app.include_router(anthropic.router)
app.include_router(deepseek.router)
app.include_router(health.router)

if __name__ == "__main__":
    # Run the FastAPI server
    port = int(os.getenv("LLM_API_PORT", "8000"))
    logger.info(f"Starting uvicorn server on port {port}")
    uvicorn.run(app, host="0.0.0.0", port=port, log_level="debug")
