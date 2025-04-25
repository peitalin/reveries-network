from pydantic import BaseModel
from typing import Optional, List, Dict, Any
from services import WEATHER_TOOLS

class QueryRequest(BaseModel):
    api_key: str
    prompt: str
    context: str
    stream: Optional[bool] = False
    tools: Optional[List[Dict[str, Any]]] = None

    @classmethod
    def with_weather_tools(cls, api_key: str, prompt: str, context: str, stream: bool = False) -> "QueryRequest":
        """
        Create a QueryRequest with pre-configured weather tools.

        Args:
            api_key: The Anthropic API key
            prompt: The user prompt
            context: The system context
            stream: Whether to stream the response

        Returns:
            A QueryRequest instance with weather tools configured
        """
        return cls(
            api_key=api_key,
            prompt=prompt,
            context=context,
            stream=stream,
            tools=WEATHER_TOOLS
        )

class QueryResponse(BaseModel):
    text: str