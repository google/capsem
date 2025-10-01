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

from .models import Agent
from .manager import SecurityManager
from .policies.debug_policy import DebugPolicy
from .models import Decision, DEFAULT_SAFE_DECISION, Verdict
from .models import DEFAULT_BLOCK_DECISION, DEFAULT_CONFIRM_DECISION

def test_init():
    "Test init and add policy"
    policy = DebugPolicy()
    manager = SecurityManager()
    manager.add_policy(policy)
    assert len(manager.get_policies()) == 1
    assert manager.get_policies()[0].name.lower() == "debug"

def test_safe_decisions_combination():
    manager = SecurityManager()

    # test all allows > return allow
    decision = manager._decide({"p1": DEFAULT_SAFE_DECISION,
                                 "p2": DEFAULT_SAFE_DECISION}, 'test')
    assert decision.verdict == Verdict.ALLOW


def test_block_decisions_combination():
    # making sure order does not matter
    decisions_grp = [{"p1": DEFAULT_SAFE_DECISION,
                       "p2": DEFAULT_BLOCK_DECISION,
                       "p3": DEFAULT_CONFIRM_DECISION},
                       {"p1": DEFAULT_BLOCK_DECISION,
                        "p2": DEFAULT_SAFE_DECISION,
                        "p3": DEFAULT_CONFIRM_DECISION},
                        {"p1": DEFAULT_BLOCK_DECISION}
                    ]
    manager = SecurityManager()
    for decisions in decisions_grp:
        decision = manager._decide(decisions, 'test')
        assert decision.verdict == Verdict.BLOCK

def test_confirm_decisions_combination():
    # making sure order does not matter
    decisions_grp = [{"p1": DEFAULT_SAFE_DECISION,
                       "p2": DEFAULT_CONFIRM_DECISION,
                       "p3": DEFAULT_CONFIRM_DECISION},
                       {"p1": DEFAULT_CONFIRM_DECISION,
                        "p2": DEFAULT_SAFE_DECISION,
                        "p3": DEFAULT_SAFE_DECISION},
                    ]
    manager = SecurityManager()
    for decisions in decisions_grp:
        decision = manager._decide(decisions, 'test')
        assert decision.verdict == Verdict.CONFIRM
