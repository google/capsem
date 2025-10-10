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

"""Identity management for multi-tenant proxy"""

import hashlib
from typing import Optional


def extract_api_key(authorization: str) -> str:
    """
    Extract API key from Authorization header.

    Args:
        authorization: Authorization header value (e.g., "Bearer sk-...")

    Returns:
        The API key

    Raises:
        ValueError: If authorization header is invalid
    """
    if not authorization:
        raise ValueError("Missing Authorization header")

    parts = authorization.split()
    if len(parts) != 2 or parts[0].lower() != "bearer":
        raise ValueError("Invalid Authorization header format. Expected: Bearer <key>")

    return parts[1]


def hash_api_key(api_key: str) -> str:
    """
    Hash API key to create deterministic user_id.

    Args:
        api_key: The API key to hash

    Returns:
        First 16 characters of SHA256 hash
    """
    hash_obj = hashlib.sha256(api_key.encode())
    return hash_obj.hexdigest()[:16]


def extract_session_id(messages: list) -> Optional[str]:
    """
    Extract session ID from message metadata if present.

    OpenAI protocol doesn't have explicit session_id, but some clients
    may include it in metadata or we can derive it from conversation.

    Args:
        messages: List of chat messages

    Returns:
        Session ID if found, None otherwise
    """
    # For now, return None - we'll implement session tracking later
    # Could be derived from conversation_id, thread_id, etc.
    return None


def get_user_id_from_auth(authorization: str) -> str:
    """
    Extract and hash API key to get user_id.

    Args:
        authorization: Authorization header value

    Returns:
        User ID (hashed API key)
    """
    api_key = extract_api_key(authorization)
    return hash_api_key(api_key)
