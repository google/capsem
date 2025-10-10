# Copyright 2025 Google LLC
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""OpenAI API endpoints"""

import uuid
import httpx
from fastapi import APIRouter, Header, HTTPException, Request
from fastapi.responses import StreamingResponse
import logging
import json

from capsem_proxy.providers.openai import OpenAIProvider
from capsem_proxy.security.identity import extract_api_key, get_user_id_from_auth, extract_session_id
from capsem_proxy.capsem_integration import security_manager, create_agent
from capsem.models import Verdict
from capsem.tools import Tool

logger = logging.getLogger(__name__)

# Initialize provider
openai_provider = OpenAIProvider()

# Create router
router = APIRouter(prefix="/v1", tags=["openai"])


@router.options("/chat/completions")
@router.options("/responses")
async def options_handler():
    """Handle OPTIONS requests (CORS preflight)"""
    return {"status": "ok"}


@router.get("/chat/completions")
@router.get("/responses")
async def get_not_supported():
    """GET method not supported - only POST"""
    raise HTTPException(
        status_code=405,
        detail="Method not allowed. Use POST for chat completions and responses."
    )


@router.post("/chat/completions")
async def chat_completion(
    request: Request,
    authorization: str = Header(..., alias="Authorization")
):
    """
    OpenAI-compatible chat completion endpoint.

    Multi-tenant: Uses client's API key from Authorization header.
    The proxy never stores API keys.
    """
    try:
        # Extract client API key
        api_key = extract_api_key(authorization)

        # Generate user_id from API key hash
        user_id = get_user_id_from_auth(authorization)

        # Generate request ID
        request_id = uuid.uuid4().hex

        # Parse request body
        try:
            body = await request.json()
        except Exception as e:
            # Log the raw body for debugging
            raw_body = await request.body()
            logger.error(f"Failed to parse JSON. Raw body: {raw_body[:500]}")
            raise ValueError(f"Invalid JSON in request body: {e}")

        # Extract session ID if present
        session_id = extract_session_id(body.get("messages", []))

        logger.info(
            f"[{request_id}] Request from user_id={user_id}, "
            f"session_id={session_id}, model={body.get('model')}"
        )

        # Create CAPSEM agent
        agent = create_agent(user_id, body.get("tools", []))

        # Extract prompt from messages
        messages = body.get("messages", [])
        prompt = "\n".join([f"{m.get('role', 'user')}: {m.get('content', '')}" for m in messages if m.get('content')])

        # CAPSEM: Check model call (prompt)
        decision = await security_manager.on_model_call(
            invocation_id=request_id,
            agent=agent,
            model_name=body.get("model", "unknown"),
            system_instructions="",
            prompt=prompt,
            media=[]
        )

        if decision.verdict == Verdict.BLOCK:
            logger.warning(f"[{request_id}] CAPSEM BLOCKED: {decision.details}")
            raise HTTPException(
                status_code=403,
                detail=f"Request blocked by security policy: {decision.details}"
            )

        # CAPSEM: Check tool calls (tool definitions)
        tools = body.get("tools", [])
        if tools:
            for tool_def in tools:
                if tool_def.get("type") == "function":
                    func = tool_def.get("function", {})
                    description = func.get("description", "") or "No description provided"
                    tool = Tool(
                        name=func.get("name", "unknown"),
                        description=description,
                        parameters=func.get("parameters", {"type": "object", "properties": {}})
                    )
                    tool_decision = await security_manager.on_tool_call(
                        invocation_id=request_id,
                        agent=agent,
                        tool=tool,
                        args={}
                    )
                    if tool_decision.verdict == Verdict.BLOCK:
                        logger.warning(f"[{request_id}] CAPSEM BLOCKED TOOL: {tool_decision.details}")
                        raise HTTPException(
                            status_code=403,
                            detail=f"Tool blocked by security policy: {tool_decision.details}"
                        )

        # Check if streaming is requested
        is_streaming = body.get("stream", False)

        if is_streaming:
            # Handle streaming response
            logger.info(f"[{request_id}] Streaming response requested")
            stream = openai_provider.chat_completion_stream(body, api_key)
            return StreamingResponse(
                stream,
                media_type="text/event-stream"
            )

        # Forward to OpenAI with client's API key (non-streaming)
        response = await openai_provider.chat_completion(body, api_key)

        # CAPSEM: Check model response
        response_content = ""
        if "choices" in response and len(response["choices"]) > 0:
            message = response["choices"][0].get("message", {})
            response_content = message.get("content", "") or ""

        response_decision = await security_manager.on_model_response(
            invocation_id=request_id,
            agent=agent,
            response=response_content,
            thoughts="",
            media=[]
        )

        if response_decision.verdict == Verdict.BLOCK:
            logger.warning(f"[{request_id}] CAPSEM BLOCKED RESPONSE: {response_decision.details}")
            raise HTTPException(
                status_code=403,
                detail=f"Response blocked by security policy: {response_decision.details}"
            )

        logger.info(f"[{request_id}] Response received")

        return response

    except HTTPException:
        # Re-raise HTTPException (includes CAPSEM blocks)
        raise
    except ValueError as e:
        logger.error(f"Invalid request: {e}")
        raise HTTPException(status_code=400, detail=str(e))
    except httpx.HTTPStatusError as e:
        logger.error(f"OpenAI API error: {e.response.status_code}")
        raise HTTPException(
            status_code=e.response.status_code,
            detail=f"Upstream provider error"
        )
    except httpx.HTTPError as e:
        logger.error(f"HTTP error: {e}")
        raise HTTPException(status_code=502, detail="Bad gateway")
    except Exception as e:
        logger.error(f"Unexpected error: {e}", exc_info=True)
        raise HTTPException(status_code=500, detail="Internal server error")


@router.post("/responses")
async def responses_create(
    request: Request,
    authorization: str = Header(..., alias="Authorization")
):
    """
    OpenAI Responses API endpoint.

    Multi-tenant: Uses client's API key from Authorization header.
    The proxy never stores API keys.
    """
    try:
        # Extract client API key
        api_key = extract_api_key(authorization)

        # Generate user_id from API key hash
        user_id = get_user_id_from_auth(authorization)

        # Generate request ID
        request_id = uuid.uuid4().hex

        # Parse request body
        body = await request.json()

        logger.info(
            f"[{request_id}] Responses API request from user_id={user_id}, "
            f"model={body.get('model')}"
        )

        # Create CAPSEM agent
        agent = create_agent(user_id, body.get("tools", []))

        # Extract prompt from input messages
        input_messages = body.get("input", [])
        prompt = "\n".join([f"{m.get('role', 'user')}: {m.get('content', '')}" for m in input_messages if m.get('content')])

        # CAPSEM: Check model call (prompt)
        decision = await security_manager.on_model_call(
            invocation_id=request_id,
            agent=agent,
            model_name=body.get("model", "unknown"),
            system_instructions="",
            prompt=prompt,
            media=[]
        )

        if decision.verdict == Verdict.BLOCK:
            logger.warning(f"[{request_id}] CAPSEM BLOCKED: {decision.details}")
            raise HTTPException(
                status_code=403,
                detail=f"Request blocked by security policy: {decision.details}"
            )

        # CAPSEM: Check tool calls (tool definitions)
        tools = body.get("tools", [])
        if tools:
            for tool_def in tools:
                if tool_def.get("type") == "function":
                    description = tool_def.get("description", "") or "No description provided"
                    tool = Tool(
                        name=tool_def.get("name", "unknown"),
                        description=description,
                        parameters=tool_def.get("parameters", {"type": "object", "properties": {}})
                    )
                    tool_decision = await security_manager.on_tool_call(
                        invocation_id=request_id,
                        agent=agent,
                        tool=tool,
                        args={}
                    )
                    if tool_decision.verdict == Verdict.BLOCK:
                        logger.warning(f"[{request_id}] CAPSEM BLOCKED TOOL: {tool_decision.details}")
                        raise HTTPException(
                            status_code=403,
                            detail=f"Tool blocked by security policy: {tool_decision.details}"
                        )

        # Forward to OpenAI with client's API key
        response = await openai_provider.responses_create(body, api_key)

        logger.info(f"[{request_id}] Response received")

        return response

    except HTTPException:
        # Re-raise HTTPException (includes CAPSEM blocks)
        raise
    except ValueError as e:
        logger.error(f"Invalid request: {e}")
        raise HTTPException(status_code=400, detail=str(e))
    except httpx.HTTPStatusError as e:
        logger.error(f"OpenAI API error: {e.response.status_code}")
        raise HTTPException(
            status_code=e.response.status_code,
            detail=f"Upstream provider error"
        )
    except httpx.HTTPError as e:
        logger.error(f"HTTP error: {e}")
        raise HTTPException(status_code=502, detail="Bad gateway")
    except Exception as e:
        logger.error(f"Unexpected error: {e}", exc_info=True)
        raise HTTPException(status_code=500, detail="Internal server error")
