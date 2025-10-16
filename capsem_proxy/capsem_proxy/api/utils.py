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

"""Shared utilities for API endpoints"""

import logging
from fastapi import HTTPException

from capsem.models import Verdict, Decision

logger = logging.getLogger(__name__)


async def decide(step: str, decision: Decision, request_id: str):
    """Handle decision from CAPSEM and log results

    Args:
        step: Name of the security check (e.g., 'on_model_call', 'on_tool_response')
        decision: Decision object from CAPSEM security manager
        request_id: Unique request identifier for logging

    Raises:
        HTTPException: If decision verdict is BLOCK (403 Forbidden)
    """
    msg = f"[{request_id}][{step}] {decision.verdict.name}: {decision.details}"
    logger.info(msg)
    print(msg)

    if decision.verdict == Verdict.BLOCK:
        raise HTTPException(
            status_code=403,
            detail=f"Request blocked by security policy: {decision.details}"
        )
