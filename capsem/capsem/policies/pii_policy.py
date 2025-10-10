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

"""PII Detection Policy using Microsoft Presidio

This policy detects and optionally blocks personally identifiable information (PII)
in prompts, tool arguments, model responses, and tool responses.

By default, uses Presidio's full capabilities with spacy NLP engine for:
- Pattern-based PII detection (EMAIL, CREDIT_CARD, SSN, etc.)
- Context-aware recognition (improved accuracy using surrounding words)
- NER-based detection (PERSON names, LOCATION, etc.)

Installation:
    uv add --group pii presidio-analyzer  # Includes spacy automatically

You can optionally configure a different NLP engine (transformers, stanza, or different
spacy model) via the nlp_engine_config parameter for custom accuracy/size tradeoffs.
"""

from typing import Optional
import json
import logging
from enum import Enum

try:
    from presidio_analyzer import AnalyzerEngine, RecognizerRegistry
    from presidio_analyzer.nlp_engine import NlpEngineProvider
    PRESIDIO_AVAILABLE = True
except ImportError:
    PRESIDIO_AVAILABLE = False
    logging.warning(
        "presidio-analyzer not available. Install with: uv add --group pii presidio-analyzer"
    )

from ..tools import Tool
from .policy import Policy
from ..models import Agent, Media, Decision, DEFAULT_SAFE_DECISION, Reason, Verdict


class PIIEntityType(str, Enum):
    """PII entity types supported by Presidio

    Pattern-based (no NLP required):
        - CREDIT_CARD, CRYPTO, EMAIL_ADDRESS, IP_ADDRESS, PHONE_NUMBER
        - US_SSN, US_BANK_NUMBER, US_DRIVER_LICENSE, US_PASSPORT, IBAN_CODE

    NLP-based (requires spacy/transformers):
        - PERSON, LOCATION, DATE_TIME
    """
    # Pattern-based entities (work without NLP)
    CREDIT_CARD = "CREDIT_CARD"
    CRYPTO = "CRYPTO"
    EMAIL_ADDRESS = "EMAIL_ADDRESS"
    IBAN_CODE = "IBAN_CODE"
    IP_ADDRESS = "IP_ADDRESS"
    PHONE_NUMBER = "PHONE_NUMBER"
    US_SSN = "US_SSN"
    US_BANK_NUMBER = "US_BANK_NUMBER"
    US_DRIVER_LICENSE = "US_DRIVER_LICENSE"
    US_PASSPORT = "US_PASSPORT"

    # NLP-based entities (require NLP engine)
    PERSON = "PERSON"
    LOCATION = "LOCATION"
    DATE_TIME = "DATE_TIME"


# Default PII entity types to check
DEFAULT_PII_ENTITIES = [
    PIIEntityType.CREDIT_CARD,
    PIIEntityType.CRYPTO,
    PIIEntityType.EMAIL_ADDRESS,
    PIIEntityType.IBAN_CODE,
    PIIEntityType.IP_ADDRESS,
    PIIEntityType.PHONE_NUMBER,
    PIIEntityType.US_SSN,
    PIIEntityType.US_BANK_NUMBER,
    PIIEntityType.US_DRIVER_LICENSE,
    PIIEntityType.US_PASSPORT,
]


class PIIPolicy(Policy):
    """Policy that detects PII in prompts, tool arguments, and responses using Microsoft Presidio

    This policy can be configured with per-entity-type decisions:
    - BLOCK: Prevent execution when detected
    - CONFIRM: Require user approval when detected
    - LOG: Log detection but allow
    - None/omitted: Pass through (don't check this entity type)

    By default, uses Presidio's spacy NLP engine (en_core_web_lg) for full accuracy
    including context-aware recognition and NER-based detection of PERSON names.

    Examples:
        # Default: Full detection with spacy (using enum for type safety)
        policy = PIIPolicy(
            entity_decisions={
                PIIEntityType.CREDIT_CARD: Verdict.BLOCK,
                PIIEntityType.US_SSN: Verdict.BLOCK,
                PIIEntityType.EMAIL_ADDRESS: Verdict.CONFIRM,
                PIIEntityType.PERSON: Verdict.LOG,  # Works with default spacy NLP
            }
        )

        # Custom NLP engine: Use transformers for best accuracy
        policy = PIIPolicy(
            entity_decisions={
                PIIEntityType.PERSON: Verdict.BLOCK,
                PIIEntityType.EMAIL_ADDRESS: Verdict.CONFIRM
            },
            nlp_engine_config={
                "nlp_engine_name": "transformers",
                "models": [{"lang_code": "en", "model_name": "dslim/bert-base-NER"}]
            }
        )

        # Block all default PII types
        policy = PIIPolicy(
            entity_decisions={entity: Verdict.BLOCK for entity in DEFAULT_PII_ENTITIES}
        )
    """

    def __init__(
        self,
        entity_decisions: Optional[dict[PIIEntityType, Verdict]] = None,
        check_prompts: bool = True,
        check_tool_args: bool = True,
        check_responses: bool = True,
        check_tool_responses: bool = True,
        score_threshold: float = 0.5,
        language: str = "en",
        nlp_engine_config: Optional[dict] = None
    ):
        """Initialize PII detection policy

        Args:
            entity_decisions: Dict mapping PIIEntityType enum to Verdict (BLOCK, CONFIRM, LOG).
                             Entities not in dict are not checked.
                             Example: {PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK}
            check_prompts: Whether to check model prompts for PII
            check_tool_args: Whether to check tool arguments for PII
            check_responses: Whether to check model responses for PII
            check_tool_responses: Whether to check tool responses for PII
            score_threshold: Confidence threshold for PII detection (0.0-1.0)
            language: Language for PII detection (default: "en")
            nlp_engine_config: Optional NLP engine configuration for Presidio.
                              If None, uses Presidio's default (spacy en_core_web_lg).
                              Example for transformers: {"nlp_engine_name": "transformers",
                                                        "models": [{"lang_code": "en",
                                                                   "model_name": "dslim/bert-base-NER"}]}
        """
        super().__init__(
            name="PIIDetection",
            description="Detects personally identifiable information using Microsoft Presidio",
            authors="CAPSEM Team",
            url="https://github.com/google/capsem/policies/pii"
        )

        if not PRESIDIO_AVAILABLE:
            raise ImportError(
                "presidio-analyzer is required for PIIPolicy. "
                "Install with: uv add --group pii presidio-analyzer"
            )

        # Default to logging all common PII if no decisions provided
        if entity_decisions is None:
            entity_decisions = {entity: Verdict.LOG for entity in DEFAULT_PII_ENTITIES}

        self.entity_decisions = entity_decisions
        self.check_prompts = check_prompts
        self.check_tool_args = check_tool_args
        self.check_responses = check_responses
        self.check_tool_responses = check_tool_responses
        self.score_threshold = score_threshold
        self.language = language
        self.nlp_engine_config = nlp_engine_config

        # Initialize Presidio analyzer
        self._init_analyzer()

        logging.info(
            f"PIIPolicy initialized - entities: {list(self.entity_decisions.keys())}"
        )

    def _init_analyzer(self):
        """Initialize Presidio analyzer engine

        If nlp_engine_config is provided, initializes with custom NLP engine.
        Otherwise, uses Presidio's default (spacy with en_core_web_lg model).
        """
        registry = RecognizerRegistry()
        registry.load_predefined_recognizers(languages=[self.language])

        if self.nlp_engine_config:
            # User provided custom NLP config (e.g., transformers, different spacy model)
            try:
                provider = NlpEngineProvider(nlp_configuration=self.nlp_engine_config)
                nlp_engine = provider.create_engine()

                self.analyzer = AnalyzerEngine(
                    nlp_engine=nlp_engine,
                    registry=registry
                )
                logging.info(
                    f"PIIPolicy initialized with custom NLP engine: {self.nlp_engine_config.get('nlp_engine_name')}"
                )
            except Exception as e:
                logging.error(f"Failed to initialize custom NLP engine: {e}")
                logging.warning("Falling back to Presidio's default (spacy)")
                self.analyzer = AnalyzerEngine(registry=registry)
        else:
            # Use Presidio's default: spacy with en_core_web_lg
            # This provides full context-aware recognition and NER-based detection
            self.analyzer = AnalyzerEngine(registry=registry)
            logging.info("PIIPolicy initialized with Presidio's default (spacy en_core_web_lg)")

    def _analyze_text(self, text: str) -> list:
        """Analyze text for PII entities

        Args:
            text: Text to analyze

        Returns:
            List of detected PII entities with scores
        """
        if not text or not text.strip():
            return []

        try:
            # Only check entity types we have decisions for
            # Convert enum values to strings for Presidio
            entities_to_check = [entity.value for entity in self.entity_decisions.keys()]

            results = self.analyzer.analyze(
                text=text,
                entities=entities_to_check,
                language=self.language,
                score_threshold=self.score_threshold
            )
            return results
        except Exception as e:
            logging.error(f"Error analyzing text for PII: {e}")
            return []

    def _make_decision(
        self,
        results: list,
        callback_name: str,
        context: str,
        is_response: bool = False
    ) -> Decision:
        """Make a decision based on detected PII and configured entity decisions

        Args:
            results: List of Presidio RecognizerResult objects
            callback_name: Name of the callback (for decision tracking)
            context: Context string for details (e.g., "prompt", "tool 'foo' arguments")
            is_response: If True, use LEAKAGE reason; otherwise SENSITIVE_DATA

        Returns:
            Decision based on most severe verdict for detected entities
        """
        if not results:
            return DEFAULT_SAFE_DECISION

        # Group detections by entity type and find most severe verdict
        detected_entities = {}
        most_severe_verdict = None
        verdict_priority = {Verdict.BLOCK: 3, Verdict.CONFIRM: 2, Verdict.LOG: 1}

        for result in results:
            entity_type = result.entity_type

            # Look up verdict by matching string value
            verdict = None
            for key, val in self.entity_decisions.items():
                if key.value == entity_type:
                    verdict = val
                    break

            if verdict is None:
                continue

            # Track detections
            if entity_type not in detected_entities:
                detected_entities[entity_type] = []
            detected_entities[entity_type].append({
                "score": result.score,
                "start": result.start,
                "end": result.end
            })

            # Update most severe verdict
            if most_severe_verdict is None or verdict_priority.get(verdict, 0) > verdict_priority.get(most_severe_verdict, 0):
                most_severe_verdict = verdict

        if not detected_entities:
            return DEFAULT_SAFE_DECISION

        # Format details
        details_parts = []
        for entity_type, detections in detected_entities.items():
            count = len(detections)
            avg_score = sum(d["score"] for d in detections) / count

            # Find the verdict for this entity type
            verdict = None
            for key, val in self.entity_decisions.items():
                if key.value == entity_type:
                    verdict = val
                    break

            action = verdict.value if verdict else "UNKNOWN"
            details_parts.append(
                f"{entity_type}(count={count}, score={avg_score:.2f}, action={action})"
            )

        details = f"PII detected in {context}: {', '.join(details_parts)}"

        # Choose reason based on context
        reason = Reason.LEAKAGE if is_response else Reason.SENSITIVE_DATA

        return Decision(
            policy=self.name,
            callback=callback_name,
            verdict=most_severe_verdict,
            reason=reason,
            details=details
        )

    async def on_model_call(
        self,
        invocation_id: str,
        agent: Agent,
        model_name: str,
        system_instructions: str,
        prompt: str,
        media: list[Media]
    ) -> Decision:
        """Check prompt for PII before sending to model"""
        if not self.check_prompts:
            return DEFAULT_SAFE_DECISION

        # Combine system instructions and prompt for analysis
        full_text = f"{system_instructions}\n{prompt}".strip()
        results = self._analyze_text(full_text)

        return self._make_decision(
            results,
            callback_name="on_model_call",
            context="prompt",
            is_response=False
        )

    async def on_tool_call(
        self,
        invocation_id: str,
        agent: Agent,
        tool: Tool,
        args: dict
    ) -> Decision:
        """Check tool arguments for PII"""
        if not self.check_tool_args:
            return DEFAULT_SAFE_DECISION

        # Convert tool arguments to text for analysis
        try:
            args_text = json.dumps(args, indent=2)
        except Exception as e:
            logging.error(f"Error serializing tool args: {e}")
            args_text = str(args)

        results = self._analyze_text(args_text)

        return self._make_decision(
            results,
            callback_name="on_tool_call",
            context=f"tool '{tool.name}' arguments",
            is_response=False
        )

    async def on_model_response(
        self,
        invocation_id: str,
        agent: Agent,
        response: str,
        thoughts: str,
        media: list[Media]
    ) -> Decision:
        """Check model response for PII"""
        if not self.check_responses:
            return DEFAULT_SAFE_DECISION

        # Analyze both response and thoughts
        full_text = f"{response}\n{thoughts}".strip()
        results = self._analyze_text(full_text)

        return self._make_decision(
            results,
            callback_name="on_model_response",
            context="model response",
            is_response=True
        )

    async def on_tool_response(
        self,
        invocation_id: str,
        agent: Agent,
        tool: Tool,
        response: dict
    ) -> Decision:
        """Check tool response for PII"""
        if not self.check_tool_responses:
            return DEFAULT_SAFE_DECISION

        # Convert tool response to text for analysis
        try:
            response_text = json.dumps(response, indent=2)
        except Exception as e:
            logging.error(f"Error serializing tool response: {e}")
            response_text = str(response)

        results = self._analyze_text(response_text)

        return self._make_decision(
            results,
            callback_name="on_tool_response",
            context=f"tool '{tool.name}' response",
            is_response=True
        )

    @classmethod
    def from_config(cls, config: dict) -> "PIIPolicy":
        """Create PIIPolicy from configuration dictionary

        Args:
            config: Configuration dictionary with keys:
                - enabled: bool (if False, returns None to skip policy)
                - entity_decisions: Dict mapping entity type strings to verdict strings
                  Example: {"EMAIL_ADDRESS": "BLOCK", "CREDIT_CARD": "CONFIRM"}
                - check_prompts: bool (optional, default True)
                - check_tool_args: bool (optional, default True)
                - check_responses: bool (optional, default True)
                - check_tool_responses: bool (optional, default True)
                - score_threshold: float (optional, default 0.5)
                - language: str (optional, default "en")

        Returns:
            Configured PIIPolicy instance, or None if disabled

        Raises:
            ValueError: If configuration is invalid
            ImportError: If presidio-analyzer not installed

        Example:
            config = {
                "enabled": True,
                "entity_decisions": {
                    "EMAIL_ADDRESS": "BLOCK",
                    "CREDIT_CARD": "CONFIRM",
                    "PERSON": "LOG"
                },
                "check_prompts": True,
                "score_threshold": 0.5
            }
            policy = PIIPolicy.from_config(config)
        """
        # Check if policy is disabled
        if not config.get("enabled", True):
            return None

        if not PRESIDIO_AVAILABLE:
            raise ImportError(
                "PIIPolicy requires presidio-analyzer. "
                "Install with: uv sync --group pii"
            )

        # Parse entity_decisions: map string entity types to enum and verdict
        entity_decisions_config = config.get("entity_decisions", {})
        if not entity_decisions_config:
            raise ValueError("PIIPolicy config must include 'entity_decisions'")

        entity_decisions = {}
        for entity_str, verdict_str in entity_decisions_config.items():
            # Convert string to PIIEntityType enum
            try:
                entity_type = PIIEntityType[entity_str]
            except KeyError:
                raise ValueError(f"Unknown PII entity type: {entity_str}")

            # Convert string to Verdict enum
            try:
                verdict = Verdict[verdict_str]
            except KeyError:
                raise ValueError(f"Unknown verdict: {verdict_str}")

            entity_decisions[entity_type] = verdict

        # Build policy arguments with defaults
        return cls(
            entity_decisions=entity_decisions,
            check_prompts=config.get("check_prompts", True),
            check_tool_args=config.get("check_tool_args", True),
            check_responses=config.get("check_responses", True),
            check_tool_responses=config.get("check_tool_responses", True),
            score_threshold=config.get("score_threshold", 0.5),
            language=config.get("language", "en"),
            nlp_engine_config=config.get("nlp_engine_config")
        )
