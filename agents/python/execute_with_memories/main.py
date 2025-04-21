import asyncio
import dotenv
import httpx
import json
import logging
import os
import requests
import uvicorn
import sys

from typing import Optional, AsyncGenerator
from fastapi import FastAPI, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import StreamingResponse
from pydantic import BaseModel

# Import route modules
from routes import anthropic, deepseek, health

# Load environment variables
dotenv.load_dotenv()

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s',
)
logger = logging.getLogger("llm-api-gateway")

# Create FastAPI app
app = FastAPI(title="LLM API Gateway")

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
    uvicorn.run(app, host="0.0.0.0", port=port, log_level="info")
