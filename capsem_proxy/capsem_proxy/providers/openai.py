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

"""OpenAI provider implementation"""

import httpx
import logging
import traceback
from typing import Dict, Any

logger = logging.getLogger(__name__)


class OpenAIProvider:
    """Proxy requests to OpenAI API"""

    def __init__(self):
        self.base_url = "https://api.openai.com/v1"

    async def chat_completion(
        self,
        request_data: Dict[str, Any],
        api_key: str
    ) -> Dict[str, Any]:
        """
        Forward chat completion request to OpenAI.

        Args:
            request_data: The chat completion request
            api_key: Client's API key (passed through, not stored)

        Returns:
            OpenAI API response
        """
        headers = {
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        }

        # Create new client per request to avoid event loop issues
        async with httpx.AsyncClient(timeout=60.0) as client:
            response = await client.post(
                f"{self.base_url}/chat/completions",
                json=request_data,
                headers=headers
            )

            response.raise_for_status()

            # Parse JSON response with better error handling
            try:
                return response.json()
            except Exception as e:
                logger.error(
                    f"Failed to parse OpenAI response as JSON. "
                    f"Status: {response.status_code}, "
                    f"Body: {response.text[:500]}, "
                    f"Error: {e}\n"
                    f"Traceback: {traceback.format_exc()}"
                )
                raise ValueError(
                    f"Failed to parse OpenAI response as JSON: {e}"
                )

    async def chat_completion_stream(
        self,
        request_data: Dict[str, Any],
        api_key: str
    ):
        """
        Forward streaming chat completion request to OpenAI.

        Args:
            request_data: The chat completion request with stream=true
            api_key: Client's API key (passed through, not stored)

        Yields:
            SSE chunks from OpenAI
        """
        headers = {
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        }

        # Create new client per request to avoid event loop issues
        async with httpx.AsyncClient(timeout=60.0) as client:
            async with client.stream(
                "POST",
                f"{self.base_url}/chat/completions",
                json=request_data,
                headers=headers
            ) as response:
                response.raise_for_status()

                # Forward SSE chunks to client
                async for chunk in response.aiter_bytes():
                    yield chunk

    async def responses_create(
        self,
        request_data: Dict[str, Any],
        api_key: str
    ) -> Dict[str, Any]:
        """
        Forward Responses API request to OpenAI.

        Args:
            request_data: The responses request
            api_key: Client's API key (passed through, not stored)

        Returns:
            OpenAI API response
        """
        headers = {
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        }

        # Create new client per request to avoid event loop issues
        async with httpx.AsyncClient(timeout=60.0) as client:
            response = await client.post(
                f"{self.base_url}/responses",
                json=request_data,
                headers=headers
            )

            response.raise_for_status()

            # Parse JSON response with better error handling
            try:
                return response.json()
            except Exception as e:
                logger.error(
                    f"Failed to parse OpenAI response as JSON. "
                    f"Status: {response.status_code}, "
                    f"Body: {response.text[:500]}, "
                    f"Error: {e}\n"
                    f"Traceback: {traceback.format_exc()}"
                )
                raise ValueError(
                    f"Failed to parse OpenAI response as JSON: {e}"
                )
