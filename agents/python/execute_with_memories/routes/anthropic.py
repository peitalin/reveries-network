from fastapi import APIRouter, HTTPException, Depends
from fastapi.responses import StreamingResponse, JSONResponse
from typing import AsyncGenerator, Dict, Any, Optional
import logging

from models import QueryRequest, QueryResponse
from services import call_anthropic
from tool_executor import process_tool_use_response, make_follow_up_request
from mcp_client import MCPClient, get_mcp_client

router = APIRouter()
logger = logging.getLogger("python-llm-server")

@router.post("/anthropic") # Define path relative to router prefix (none here)
async def query_anthropic_route(
    request: QueryRequest,
    mcp_client: Optional[MCPClient] = Depends(get_mcp_client)
):
    logger.info(f"Received request to /anthropic endpoint (stream={request.stream}, tools_provided={bool(request.tools)}, mcp_connected={bool(mcp_client)})" )
    try:
        # Make the initial call to Anthropic
        response_data = await call_anthropic(
            request.prompt,
            request.context,
            stream=request.stream,
            tools=request.tools
        )

        if isinstance(response_data, dict):
            response_data["messages"] = [{
                "role": "user",
                "content": request.prompt
            }]

        if request.stream:
            # Streaming responses don't support tool use yet
            if isinstance(response_data, AsyncGenerator):
                return StreamingResponse(response_data, media_type="text/event-stream")
            else:
                logger.error("Expected an async generator for streaming, but got %s.", type(response_data).__name__)
                raise HTTPException(status_code=500, detail="Internal server error: Streaming failed.")
        else:
            # For non-streaming responses, check if we need to handle tool use
            if isinstance(response_data, Dict) and response_data.get("stop_reason") == "tool_use":
                logger.info("Detected tool use response. Processing tools...")

                # Process the tool use response
                try:
                    # 1. Parse the tool use block and execute the tool using the injected mcp_client
                    tool_use_id, tool_name, tool_result = await process_tool_use_response(
                        response_data,
                        mcp_client=mcp_client
                    )

                    if not tool_use_id:
                        logger.error("Failed to extract tool use information")
                        return JSONResponse(
                            content={"error": "Failed to process tool use response",
                                    "original_response": response_data}
                        )

                    logger.info(f"Tool '{tool_name}' executed successfully. Making follow-up request.")
                    logger.info(f"Tool result: {tool_result}")

                    # 2. Make the follow-up request with the tool result
                    follow_up_response = await make_follow_up_request(
                        original_response=response_data,
                        tool_use_id=tool_use_id,
                        tool_result=tool_result,
                        tools=request.tools
                    )

                    # 3. Check if the follow-up response is also a tool use
                    if follow_up_response.get("stop_reason") == "tool_use":
                        logger.info("Follow-up response also contains tool use. Returning as-is.")
                        follow_up_response["messages"] = response_data["messages"]
                        follow_up_response["messages"].append(response_data["content"])
                        follow_up_response["messages"].append({
                             "role": "user",
                             "content": [{
                                 "type": "tool_result",
                                 "tool_use_id": tool_use_id,
                                 "content": tool_result
                              }]
                        })
                        return JSONResponse(content=follow_up_response)
                    else:
                        # Extract the final text from the follow-up response
                        final_text = "".join([
                            content["text"]
                            for content in follow_up_response.get("content", [])
                            if content.get("type") == "text"
                        ])
                        logger.info(f"Returning final response after tool use: '{final_text[:50]}...'")
                        return QueryResponse(text=final_text)

                except Exception as e:
                    logger.exception(f"Error during tool execution flow: {e}")
                    # If tool execution fails, return the original tool use response
                    return JSONResponse(
                        content={"error": f"Tool execution failed: {str(e)}",
                                "original_response": response_data}
                    )
            elif isinstance(response_data, str):
                # Standard text response (no tool use)
                logger.info("Returning standard text response.")
                return QueryResponse(text=response_data)
            elif isinstance(response_data, Dict):
                # Other dictionary responses that aren't tool use
                logger.info("Returning raw response dictionary.")
                return JSONResponse(content=response_data)
            else:
                logger.error("Expected a string or dict for non-streaming response, but got %s.", type(response_data).__name__)
                raise HTTPException(status_code=500, detail="Internal server error: Invalid response format.")

    except HTTPException as e:
        logger.error(f"HTTPException in /anthropic route: {e.detail} (status: {e.status_code})")
        raise e # Re-raise exceptions from the service layer or call_anthropic
    except Exception as e:
        logger.exception("Unhandled error processing /anthropic request in route")
        raise HTTPException(status_code=500, detail=f"Internal Server Error: {str(e)}")