from fastapi import APIRouter, HTTPException
from fastapi.responses import StreamingResponse
from typing import AsyncGenerator
import logging

# Import models and services from sibling modules
from ..models import QueryRequest, QueryResponse
from ..services import call_anthropic

router = APIRouter()
logger = logging.getLogger("llm-api-gateway") # Use the same logger name

@router.post("/anthropic") # Define path relative to router prefix (none here)
async def query_anthropic_route(request: QueryRequest):
    logger.info(f"Received request to /anthropic endpoint (stream={request.stream})")
    try:
        response_data = await call_anthropic(
            request.api_key,
            request.prompt,
            request.context,
            stream=request.stream
        )

        if request.stream:
            if isinstance(response_data, AsyncGenerator):
                return StreamingResponse(response_data, media_type="text/event-stream")
            else:
                logger.error("Expected an async generator for streaming, but got something else.")
                raise HTTPException(status_code=500, detail="Internal server error: Streaming failed.")
        else:
             if isinstance(response_data, str):
                return QueryResponse(text=response_data)
             else:
                logger.error("Expected a string for non-streaming response, but got something else.")
                raise HTTPException(status_code=500, detail="Internal server error: Invalid response format.")

    except HTTPException as e:
        raise e # Re-raise exceptions from the service layer
    except Exception as e:
        logger.error(f"Error processing /anthropic request in route: {str(e)}")
        raise HTTPException(status_code=500, detail=str(e))