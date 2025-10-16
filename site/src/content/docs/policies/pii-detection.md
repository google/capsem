---
title: PII Detection Policy
description: Detect and control personally identifiable information (PII) in agent
  interactions using Microsoft Presidio
---
Detect and control personally identifiable information (PII) in agent interactions using Microsoft Presidio

The PII Detection Policy uses [Microsoft Presidio](https://github.com/microsoft/presidio) to detect and control personally identifiable information (PII) in agent interactions.

## Features

- **Configurable per-entity decisions**: Choose BLOCK, CONFIRM, or LOG for each PII type
- **Multi-context checking**: Scan prompts, tool arguments, model responses, and tool responses
- **Full NLP capabilities**: Uses spacy by default for context-aware and NER-based detection
- **Flexible NLP engines**: Switch to transformers, stanza, or smaller spacy models
- **Framework-agnostic**: Works with any CAPSEM-compatible agent framework

## Installation

### Standard Installation


```python
# Install CAPSEM with PII support
!uv add --group pii presidio-analyzer
```

## Quick Start

### Basic Usage


```python
from capsem.policies.pii_policy import PIIPolicy, PIIEntityType
from capsem.models import Verdict

# Uses Presidio's default (spacy en_core_web_lg)
# Detects pattern-based PII + PERSON names with NER
# Use PIIEntityType enum for type safety and IDE autocomplete
policy = PIIPolicy(
    entity_decisions={
        PIIEntityType.CREDIT_CARD: Verdict.BLOCK,
        PIIEntityType.US_SSN: Verdict.BLOCK,
        PIIEntityType.EMAIL_ADDRESS: Verdict.CONFIRM,
        PIIEntityType.PERSON: Verdict.LOG,  # Works with default NLP
    }
)

print(f"Policy created with {len(policy.entity_decisions)} entity types configured")
```

## Configuration

### Entity Decisions

Map each PII entity type to a verdict using the `PIIEntityType` enum:

- **`Verdict.BLOCK`**: Prevent execution/block results
- **`Verdict.CONFIRM`**: Require user approval
- **`Verdict.LOG`**: Log warning but allow
- **Omitted**: Don't check this entity type


```python
from capsem.policies.pii_policy import PIIPolicy, PIIEntityType, DEFAULT_PII_ENTITIES
from capsem.models import Verdict

# Block all default PII types
policy_strict = PIIPolicy(
    entity_decisions={entity: Verdict.BLOCK for entity in DEFAULT_PII_ENTITIES}
)

# Custom per-type decisions (recommended: use enum)
policy_custom = PIIPolicy(
    entity_decisions={
        PIIEntityType.CREDIT_CARD: Verdict.BLOCK,      # Block credit cards
        PIIEntityType.US_SSN: Verdict.BLOCK,           # Block SSNs
        PIIEntityType.EMAIL_ADDRESS: Verdict.CONFIRM,  # Ask user about emails
        PIIEntityType.PHONE_NUMBER: Verdict.LOG,       # Just log phone numbers
        # PIIEntityType.IP_ADDRESS not listed = not checked
    }
)

print(f"Strict policy: {len(policy_strict.entity_decisions)} types configured")
print(f"Custom policy: {len(policy_custom.entity_decisions)} types configured")
```

### Selective Checking

Control which contexts are checked:


```python
# Check all contexts
policy_all = PIIPolicy(
    entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK},
    check_prompts=True,          # Check user prompts
    check_tool_args=True,        # Check tool arguments
    check_responses=True,        # Check model responses
    check_tool_responses=True,   # Check tool responses
)

# Example: Only check outgoing data (responses)
policy_dlp = PIIPolicy(
    entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK},
    check_prompts=False,
    check_tool_args=False,
    check_responses=True,        # Check model output
    check_tool_responses=True,   # Check tool output
)

print("DLP policy: Only checks outgoing data (responses)")
```

### Detection Threshold

Control sensitivity with `score_threshold` (0.0-1.0):


```python
# Default threshold (balanced)
policy_balanced = PIIPolicy(
    entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK},
    score_threshold=0.5  # Default: 0.5
)

# Strict (fewer false positives, may miss some)
policy_strict = PIIPolicy(
    entity_decisions={PIIEntityType.PERSON: Verdict.BLOCK},
    score_threshold=0.8
)

# Lenient (catches more, more false positives)
policy_lenient = PIIPolicy(
    entity_decisions={PIIEntityType.PERSON: Verdict.BLOCK},
    score_threshold=0.3
)

print("Thresholds: Strict=0.8, Balanced=0.5, Lenient=0.3")
```

## NLP Engine Configuration

### Using Different spaCy Models


```python
# Small model (12MB, fast)
policy_sm = PIIPolicy(
    entity_decisions={PIIEntityType.PERSON: Verdict.BLOCK},
    nlp_engine_config={
        "nlp_engine_name": "spacy",
        "models": [{"lang_code": "en", "model_name": "en_core_web_sm"}]
    }
)

# Large model (560MB, best accuracy) - default
policy_lg = PIIPolicy(
    entity_decisions={PIIEntityType.PERSON: Verdict.BLOCK},
    nlp_engine_config={
        "nlp_engine_name": "spacy",
        "models": [{"lang_code": "en", "model_name": "en_core_web_lg"}]
    }
)

print("Available spaCy models: sm (12MB), md (40MB), lg (560MB)")
```

### Using Transformers for Best Accuracy


```python
# Requires: uv add transformers torch
policy_transformers = PIIPolicy(
    entity_decisions={PIIEntityType.PERSON: Verdict.BLOCK},
    nlp_engine_config={
        "nlp_engine_name": "transformers",
        "models": [{"lang_code": "en", "model_name": "dslim/bert-base-NER"}]
    }
)

print("Transformers: Best accuracy, ~50ms latency")
```

## Use Cases

### GDPR Compliance


```python
# GDPR compliance: Block all PII in EU region
from capsem.policies.pii_policy import PIIPolicy, PIIEntityType, DEFAULT_PII_ENTITIES
from capsem.models import Verdict

gdpr_policy = PIIPolicy(
    entity_decisions={entity: Verdict.BLOCK for entity in DEFAULT_PII_ENTITIES},
    check_prompts=True,
    check_responses=True,
)

print(f"GDPR policy: Blocking {len(gdpr_policy.entity_decisions)} PII entity types")
```

### Data Loss Prevention (DLP)


```python
# Prevent sensitive data leakage in model responses
from capsem.policies.pii_policy import PIIPolicy, PIIEntityType
from capsem.models import Verdict

dlp_policy = PIIPolicy(
    entity_decisions={
        PIIEntityType.CREDIT_CARD: Verdict.BLOCK,
        PIIEntityType.US_SSN: Verdict.BLOCK,
        PIIEntityType.US_BANK_NUMBER: Verdict.BLOCK,
    },
    check_prompts=False,      # Allow in prompts
    check_responses=True,     # Block in responses (DLP)
    check_tool_responses=True,
)

print("DLP policy: Prevents sensitive data leakage in outputs only")
```

### Customer Service Safety


```python
# Require confirmation for PII in customer interactions
from capsem.policies.pii_policy import PIIPolicy, PIIEntityType
from capsem.models import Verdict

cs_policy = PIIPolicy(
    entity_decisions={
        PIIEntityType.CREDIT_CARD: Verdict.BLOCK,     # Never allow
        PIIEntityType.EMAIL_ADDRESS: Verdict.CONFIRM, # Ask first
        PIIEntityType.PHONE_NUMBER: Verdict.CONFIRM,  # Ask first
        PIIEntityType.PERSON: Verdict.LOG,            # Just log
    },
    nlp_engine_config={
        "nlp_engine_name": "spacy",
        "models": [{"lang_code": "en", "model_name": "en_core_web_sm"}]
    }
)

print("Customer Service policy: Blocks cards, confirms emails/phones, logs names")
```

## Integration with Security Manager


```python
from capsem import SecurityManager
from capsem.policies.pii_policy import PIIPolicy, PIIEntityType
from capsem.models import Verdict

# Create PII policy
pii_policy = PIIPolicy(
    entity_decisions={
        PIIEntityType.CREDIT_CARD: Verdict.BLOCK,
        PIIEntityType.EMAIL_ADDRESS: Verdict.CONFIRM,
        PIIEntityType.PHONE_NUMBER: Verdict.LOG,
    }
)

# Add to security manager
security_manager = SecurityManager()
security_manager.add_policy(pii_policy)

print(f"Security manager configured with {len(security_manager.policies)} policy")
```

## References

- [Microsoft Presidio Documentation](https://microsoft.github.io/presidio/)
- [Presidio Supported Entities](https://microsoft.github.io/presidio/supported_entities/)
- [Presidio Analyzer Architecture](https://microsoft.github.io/presidio/analyzer/)
