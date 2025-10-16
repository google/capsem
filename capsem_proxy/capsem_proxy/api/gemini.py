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

"""Google Gemini API endpoints"""

from urllib import response
import uuid
import httpx
from fastapi import APIRouter, Header, HTTPException, Request
from fastapi.responses import StreamingResponse
import logging

from google.genai.types import Content, Part, GenerateContentResponse
from capsem_proxy.providers.gemini import GeminiProvider
from capsem_proxy.security.identity import get_user_id_from_auth
from capsem_proxy.capsem_integration import security_manager, create_agent
from capsem.models import Verdict, Media, Decision
from capsem.tools import Tool

logger = logging.getLogger(__name__)

# Initialize provider
gemini_provider = GeminiProvider()

# Create router
router = APIRouter(prefix="/v1beta", tags=["gemini"])


def extract_gemini_api_key(x_goog_api_key: str) -> str:
    """Extract API key from x-goog-api-key header"""
    if not x_goog_api_key:
        raise ValueError("Missing x-goog-api-key header")
    return x_goog_api_key


@router.options("/models/{model}:generateContent")
@router.options("/models/{model}:streamGenerateContent")
async def options_handler(model: str):
    """Handle OPTIONS requests (CORS preflight)"""
    return {"status": "ok"}

async def _decide(step: str, decision: Decision, request_id: str):
    """Handle decision from CAPSEM and log results"""

    msg = f"[{request_id}][{step}]{decision.verdict.name}:{decision.details}"
    logger.info(msg)
    print(msg)
    if decision.verdict == Verdict.BLOCK:
        raise HTTPException(status_code=403,
                            detail=f"Request blocked by security policy: {decision.details}",
                            )

@router.post("/models/{model}:generateContent")
async def generate_content(
    model: str, request: Request, x_goog_api_key: str = Header(...)
):
    """
    Gemini generateContent endpoint with CAPSEM security.

    Multi-tenant: Uses client's API key from x-goog-api-key header.
    The proxy never stores API keys.
    """
    try:
        # Extract client API key
        api_key = extract_gemini_api_key(x_goog_api_key)

        # Generate user_id from API key hash (use same identity system as OpenAI)
        user_id = get_user_id_from_auth(f"Bearer {api_key}")

        # Generate request ID
        request_id = uuid.uuid4().hex

        # Parse request body
        try:
            body = await request.json()
        except Exception as e:
            logger.error(f"Failed to parse JSON: {e}")
            raise ValueError(f"Invalid JSON in request body: {e}")

        logger.info(f"[{request_id}] Gemini request from user_id={user_id}, model={model}")

        # Create CAPSEM agent
        # Note: Gemini uses "tools" in request, similar structure to OpenAI
        agent = create_agent(user_id, body.get("tools", []))


        # ON_MODEL_CALL
        # Extract prompt from contents
        contents = body.get("contents", [])

        prompt_parts = []
        thoughts_parts = []
        for content in contents:
            try:
                content = Content(**content)  # Validate structure
            except Exception as e:
                logger.warning(f"Invalid content structure: {e}")
                continue
            if not content.parts:
                continue
            for part in content.parts:
                if part.text:
                    if part.thought:
                        thoughts_parts.append(part.text)
                    else:
                        prompt_parts.append(part.text)

            # on tool call response
            if part.function_response:
                decision = await security_manager.on_tool_response(
                    invocation_id=request_id,
                    agent=agent,
                    tool=Tool(
                        name=part.function_response.name,
                        description="Function response from model",
                        parameters={}),
                    response=part.function_response.response or {},
                )
                await _decide('on_tool_response', decision, request_id)


        # check the model call
        prompt = "\n".join(prompt_parts)
        thoughts = "\n".join(thoughts_parts)
        # CAPSEM: Check model call (prompt)
        decision = await security_manager.on_model_call(
            invocation_id=request_id,
            agent=agent,
            model_name=model,
            system_instructions="",
            prompt=prompt,
            media=[],
        )
        await _decide('on_model_call', decision, request_id)


        # Forward to Gemini
        raw_response = await gemini_provider.generate_content(model, body, api_key)
        try:
            response = GenerateContentResponse(**raw_response)
        except Exception as e:
            logger.error(f"Invalid response structure from Gemini: {e}")
            raise HTTPException(status_code=502, detail="Invalid response from provider")


        # CAPSEM: Check model response
        response_parts = []
        response_thoughts = []
        if not response.candidates:
            return response  # No candidates to process
        for candidate in response.candidates:
            if not candidate.content:
                continue
            if not candidate.content.parts or not len(candidate.content.parts):
                continue
            for part in candidate.content.parts:
                if part.text:
                    if part.thought:
                        response_thoughts.append(part.text)
                    else:
                        response_parts.append(part.text)

                # analyze function call
                if part.function_call:

                    # async def on_tool_call(self, invocation_id: str, agent: Agent, tool: Tool, args: dict) -> Decision:
                    tool_decision = await security_manager.on_tool_call(
                        invocation_id=request_id,
                        agent=agent,
                        tool=Tool(
                            name=part.function_call.name,
                            description="Function call from model",
                            parameters=part.function_call.args or {},
                        ),
                        args=part.function_call.args or {},
                    )
                    await _decide('on_tool_call', tool_decision, request_id)

        response_text = "\n".join(response_parts)
        response_thoughts = "\n".join(response_thoughts)

        response_decision = await security_manager.on_model_response(
            invocation_id=request_id,
            agent=agent,
            response=response_text,
            thoughts=response_thoughts,
            media=[],  # Fixme
        )
        await _decide('on_model_response', response_decision, request_id)
        logger.info(f"[{request_id}] Response received, returning to client")

        # logger.debug(f"[{request_id}] Response type: {type(response)}, keys: {response.keys() if isinstance(response, dict) else 'not a dict'}")
        return response

    except HTTPException:
        raise
    except ValueError as e:
        logger.error(f"Invalid request: {e}")
        raise HTTPException(status_code=400, detail=str(e))
    except httpx.HTTPStatusError as e:
        logger.error(f"Gemini API error: {e.response.status_code}")
        raise HTTPException(
            status_code=e.response.status_code, detail="Upstream provider error"
        )
    except httpx.HTTPError as e:
        logger.error(f"HTTP error: {e}")
        raise HTTPException(status_code=502, detail="Bad gateway")
    except Exception as e:
        logger.error(f"Unexpected error: {e}", exc_info=True)
        raise HTTPException(status_code=500, detail="Internal server error")


@router.post("/models/{model}:streamGenerateContent")
async def stream_generate_content(
    model: str, request: Request, x_goog_api_key: str = Header(...)
):
    """
    Gemini streamGenerateContent endpoint with CAPSEM security.

    Multi-tenant: Uses client's API key from x-goog-api-key header.
    """
    try:
        # Extract client API key
        api_key = extract_gemini_api_key(x_goog_api_key)

        # Generate user_id from API key hash
        user_id = get_user_id_from_auth(f"Bearer {api_key}")

        # Generate request ID
        request_id = uuid.uuid4().hex

        # Parse request body
        body = await request.json()

        logger.info(
            f"[{request_id}] Gemini streaming request from user_id={user_id}, model={model}"
        )

        # Create CAPSEM agent
        agent = create_agent(user_id, body.get("tools", []))

        # Extract prompt from contents
        contents = body.get("contents", [])
        prompt_parts = []
        for content in contents:
            role = content.get("role", "user")
            parts = content.get("parts", [])
            for part in parts:
                if "text" in part:
                    prompt_parts.append(f"{role}: {part['text']}")

        prompt = "\n".join(prompt_parts)

        # CAPSEM: Check model call
        decision = await security_manager.on_model_call(
            invocation_id=request_id,
            agent=agent,
            model_name=model,
            system_instructions="",
            prompt=prompt,
            media=[],
        )

        if decision.verdict == Verdict.BLOCK:
            logger.warning(f"[{request_id}] CAPSEM BLOCKED: {decision.details}")
            raise HTTPException(
                status_code=403,
                detail=f"Request blocked by security policy: {decision.details}",
            )

        # CAPSEM: Check tools if present
        tools = body.get("tools", [])
        if tools:
            for tool_def in tools:
                if "functionDeclarations" in tool_def:
                    for func_decl in tool_def["functionDeclarations"]:
                        tool = Tool(
                            name=func_decl.get("name", "unknown"),
                            description=func_decl.get("description", "")
                            or "No description provided",
                            parameters=func_decl.get(
                                "parameters", {"type": "object", "properties": {}}
                            ),
                        )
                        # FIXME args is not available here
                        tool_decision = await security_manager.on_tool_call(
                            invocation_id=request_id, agent=agent, tool=tool, args={}
                        )
                        if tool_decision.verdict == Verdict.BLOCK:
                            logger.warning(
                                f"[{request_id}] CAPSEM BLOCKED TOOL: {tool_decision.details}"
                            )
                            raise HTTPException(
                                status_code=403,
                                detail=f"Tool blocked by security policy: {tool_decision.details}",
                            )

        # Forward to Gemini (streaming)
        logger.info(f"[{request_id}] Streaming response requested")
        stream = gemini_provider.generate_content_stream(model, body, api_key)
        return StreamingResponse(stream, media_type="text/event-stream")

    except HTTPException:
        raise
    except ValueError as e:
        logger.error(f"Invalid request: {e}")
        raise HTTPException(status_code=400, detail=str(e))
    except httpx.HTTPError as e:
        logger.error(f"HTTP error: {e}")
        raise HTTPException(status_code=502, detail="Bad gateway")
    except Exception as e:
        logger.error(f"Unexpected error: {e}", exc_info=True)
        raise HTTPException(status_code=500, detail="Internal server error")
