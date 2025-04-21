from pydantic import BaseModel
from typing import Optional

class QueryRequest(BaseModel):
    api_key: str
    prompt: str
    context: str
    stream: Optional[bool] = False

class QueryResponse(BaseModel):
    text: str