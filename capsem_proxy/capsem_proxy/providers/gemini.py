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

"""Google Gemini API provider - transparent HTTP proxy"""

from typing import Dict, Any, AsyncGenerator
import httpx
import logging

logger = logging.getLogger(__name__)


class GeminiProvider:
    """Provider for Google Gemini API - forwards requests to Google"""

    def __init__(self):
        self.base_url = "https://generativelanguage.googleapis.com/v1beta"

    async def generate_content(
        self, model: str, request_data: Dict[str, Any], api_key: str
    ) -> Dict[str, Any]:
        """
        Forward generateContent request to Gemini API.

        Args:
            model: Model name (e.g., "gemini-2.0-flash-exp")
            request_data: Request body with contents, generationConfig, etc.
            api_key: Gemini API key

        Returns:
            Gemini API response
        """
        async with httpx.AsyncClient(timeout=60.0) as client:
            response = await client.post(
                f"{self.base_url}/models/{model}:generateContent",
                json=request_data,
                headers={"x-goog-api-key": api_key, "Content-Type": "application/json"},
            )
            response.raise_for_status()
            return response.json()

    async def generate_content_stream(
        self, model: str, request_data: Dict[str, Any], api_key: str
    ) -> AsyncGenerator[bytes, None]:
        """
        Forward streaming generateContent request to Gemini API.

        Args:
            model: Model name
            request_data: Request body
            api_key: Gemini API key

        Yields:
            SSE chunks from Gemini
        """
        async with httpx.AsyncClient(timeout=60.0) as client:
            async with client.stream(
                "POST",
                f"{self.base_url}/models/{model}:streamGenerateContent",
                json=request_data,
                headers={"x-goog-api-key": api_key, "Content-Type": "application/json"},
            ) as response:
                response.raise_for_status()

                # Forward SSE chunks to client
                async for chunk in response.aiter_bytes():
                    yield chunk
