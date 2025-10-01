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

from uuid import uuid4
from typing import Any, AsyncGenerator
from google.adk.agents.run_config import RunConfig
from google.adk.events.event import Event, EventActions
from google.adk.runners import Runner
from google.genai import types
from typing import Optional
import warnings
from ...models import Decision, Verdict

class CAPSEMRunner(Runner):
    """
    A custom Runner that inherits from ADKRunner and integrate with CAPSEM
    via a shared context variable "capsem" to enable blocking unsafe flows.
    """

    async def run_async(self, *,
                        user_id: str,
                        session_id: str,
                        new_message: types.Content,
                        state_delta: Optional[dict[str, Any]] = None,
                        run_config: Optional[RunConfig] = None,
    ) -> AsyncGenerator[Event, None]:
        """ Check CAPSEM decision in session state and interupt execution if
        needed.
        """

        # track which decision id was displayed last as we can't clear session
        # from the runner
        last_decision_id: str = ""
        # Get the original asynchronous generator from the parent class
        event_stream = super().run_async(user_id=user_id,
                                        session_id=session_id,
                                        new_message=new_message,
                                        state_delta=state_delta,
                                        run_config=run_config)
        async for event in event_stream:

            # get session context
            adk_session = await self.session_service.get_session(app_name=self.app_name,
                                                                 user_id=user_id,
                                                                 session_id=session_id)

            # print('here', adk_session.state)
            # get CAPSEM composed decision
            if adk_session and adk_session.state:
                decision = adk_session.state.get("capsem")
                try:
                    decision = Decision.model_validate(decision)
                except Exception as e:
                    warnings.warn(f"Capsem result is not decision object: {type(decision)}")
                    yield event
                    continue

                # check if we have seen this decision already
                already_seen = decision.id == last_decision_id

                # if new decision
                if not already_seen:
                    last_decision_id = decision.id
                    print(decision)

                if decision.verdict == Verdict.BLOCK:
                    # If CAPSEM decided to block, we skip all events
                    # and yield a final "escalation" event.
                    security_violation_event = Event(
                        invocation_id=uuid4().hex,
                        author=f'capsem_policy:{decision.policy}',
                        error_code=decision.reason.value,
                        error_message=decision.details,
                    )
                    yield security_violation_event
                    return
                # avoid reconfirming multiple times
                elif decision.verdict == Verdict.CONFIRM and not already_seen:
                    # FIXME: trigger user confirmation flow
                    warnings.warn("User confirmation flow not implemented yet.")
                    yield event
                elif decision.verdict == Verdict.ALLOW:
                    yield event
            else:
                # warnings.warn("No CAPSEM decision found in session state.")
                yield event