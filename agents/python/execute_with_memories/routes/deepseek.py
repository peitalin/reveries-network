from fastapi import APIRouter, HTTPException
import logging

# Use absolute imports
from models import QueryRequest, QueryResponse
from services import call_deepseek

router = APIRouter()
logger = logging.getLogger("llm-api-gateway")

@router.post("/deepseek", response_model=QueryResponse)
async def query_deepseek_route(request: QueryRequest):
    logger.info("Received request to /deepseek endpoint")
    try:
        # Note: call_deepseek is currently synchronous
        response = call_deepseek(
            request.api_key,
            request.prompt,
            request.context
        )
        return QueryResponse(text=response)
    except HTTPException as e:
        raise e # Re-raise exceptions from the service layer
    except Exception as e:
        logger.error(f"Error processing /deepseek request in route: {str(e)}")
        raise HTTPException(status_code=500, detail=str(e))