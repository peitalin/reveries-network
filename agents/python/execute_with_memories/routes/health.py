from fastapi import APIRouter
import logging
import os

router = APIRouter()
logger = logging.getLogger("python-llm-server")

@router.get("/health")
async def health_check_route():
    logger.info("Health check request received")
    return {
        "status": "ok",
        "pid": os.getpid(),
    }