import requests
import json
import os
import logging
from fastapi import FastAPI, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel
import uvicorn
from dotenv import load_dotenv

# Load environment variables
load_dotenv()
# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s',
)
logger = logging.getLogger("llm-api-gateway")

# Create FastAPI app
app = FastAPI(title="LLM API Gateway")
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

class QueryRequest(BaseModel):
    api_key: str
    prompt: str
    context: str

class QueryResponse(BaseModel):
    text: str

def call_anthropic(api_key, prompt, context):
    """
    Makes a request to Anthropic's Claude API

    Args:
        api_key (str): Anthropic API key
        prompt (str): The question or prompt to send to the model
        context (str): The system context or instructions

    Returns:
        str: The model's response text
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
    logger.info(f"Anthropic response body: {response_data}")

    text = "".join([content["text"] for content in response_data["content"]
                   if content["type"] == "text"])

    logger.info(f"Received response from Anthropic API: '{text[:30]}...'")
    return text

def call_deepseek(api_key, prompt, context):
    """
    Makes a request to DeepSeek's Chat API

    Args:
        api_key (str): DeepSeek API key
        prompt (str): The question or prompt to send to the model
        context (str): The system context or instructions

    Returns:
        str: The model's response text
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

    response = requests.post(url, headers=headers, json=payload)

    if response.status_code != 200:
        logger.error(f"DeepSeek API error: {response.text}")
        raise Exception(f"DeepSeek API error: {response.text}")

    response_data = response.json()
    text = response_data["choices"][0]["message"]["content"]

    logger.info(f"Received response from DeepSeek API: '{text[:30]}...'")
    return text

@app.post("/anthropic", response_model=QueryResponse)
async def query_anthropic(request: QueryRequest):
    logger.info("Received request to /anthropic endpoint")
    try:
        response = call_anthropic(
            request.api_key,
            request.prompt,
            request.context
        )
        return {"text": response}
    except Exception as e:
        logger.error(f"Error processing /anthropic request: {str(e)}")
        raise HTTPException(status_code=500, detail=str(e))

@app.post("/deepseek", response_model=QueryResponse)
async def query_deepseek(request: QueryRequest):
    logger.info("Received request to /deepseek endpoint")
    try:
        response = call_deepseek(
            request.api_key,
            request.prompt,
            request.context
        )
        return {"text": response}
    except Exception as e:
        logger.error(f"Error processing /deepseek request: {str(e)}")
        raise HTTPException(status_code=500, detail=str(e))

@app.get("/health")
async def health_check():
    logger.info("Health check request received")
    return {
        "status": "ok",
        "pid": os.getpid(),
    }

if __name__ == "__main__":
    # Run the FastAPI server
    port = int(os.getenv("LLM_API_PORT", "8000"))
    print(f"Starting uvicorn server on port {port}")
    uvicorn.run(app, host="0.0.0.0", port=port, log_level="info")
