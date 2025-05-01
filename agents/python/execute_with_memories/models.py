from pydantic import BaseModel
from typing import Optional, List, Dict, Any
from services import WEATHER_TOOLS

class QueryRequest(BaseModel):
    prompt: str
    context: str
    # api_key: Optional[str] = None # No longer needed from client
    stream: Optional[bool] = False
    tools: Optional[List[Dict[str, Any]]] = None

class QueryResponse(BaseModel):
    text: str