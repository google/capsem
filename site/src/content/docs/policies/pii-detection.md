---
title: PII Detection Policy
description: Detect and control personally identifiable information (PII) in agent interactions using Microsoft Presidio
---

The PII Detection Policy uses [Microsoft Presidio](https://github.com/microsoft/presidio) to detect and control personally identifiable information (PII) in agent interactions.

## Features

- **Configurable per-entity decisions**: Choose BLOCK, CONFIRM, or LOG for each PII type
- **Multi-context checking**: Scan prompts, tool arguments, model responses, and tool responses
- **Full NLP capabilities**: Uses spacy by default for context-aware and NER-based detection
- **Flexible NLP engines**: Switch to transformers, stanza, or smaller spacy models
- **Framework-agnostic**: Works with any CAPSEM-compatible agent framework

## Installation

### Standard Installation

```bash
# Install CAPSEM with PII support (includes spacy automatically)
uv add --group pii presidio-analyzer

# Or with pip
pip install presidio-analyzer

# Download the spacy model (required, 560MB)
python -m spacy download en_core_web_lg
```

**What you get:**
- Pattern-based PII detection (EMAIL, CREDIT_CARD, SSN, etc.)
- Context-aware recognition (improved accuracy)
- NER-based detection (PERSON names, LOCATION, etc.)
- Full Presidio capabilities with spacy NLP engine

**Note:** The `presidio-analyzer` package includes `spacy` as a dependency, so you don't need to install it separately. You just need to download the language model.

### Alternative NLP Engines (Optional)

Want different accuracy/performance tradeoffs? Install alternative engines:

```bash
# Option 1: Smaller spaCy model (faster, less accurate)
python -m spacy download en_core_web_sm  # 12MB instead of 560MB

# Option 2: Transformers (best accuracy, slowest)
uv add transformers torch

# Option 3: Stanza
uv add stanza
```

## Quick Start

### Basic Usage (Default spaCy)

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
```

### With Custom NLP Engine

```python
# Use transformers for best accuracy
policy = PIIPolicy(
    entity_decisions={
        PIIEntityType.PERSON: Verdict.BLOCK,
        PIIEntityType.EMAIL_ADDRESS: Verdict.CONFIRM,
    },
    nlp_engine_config={
        "nlp_engine_name": "transformers",
        "models": [{"lang_code": "en", "model_name": "dslim/bert-base-NER"}]
    }
)

# Or use smaller spacy model for speed
policy = PIIPolicy(
    entity_decisions={PIIEntityType.PERSON: Verdict.BLOCK},
    nlp_engine_config={
        "nlp_engine_name": "spacy",
        "models": [{"lang_code": "en", "model_name": "en_core_web_sm"}]
    }
)
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
policy = PIIPolicy(
    entity_decisions={entity: Verdict.BLOCK for entity in DEFAULT_PII_ENTITIES}
)

# Custom per-type decisions (recommended: use enum)
policy = PIIPolicy(
    entity_decisions={
        PIIEntityType.CREDIT_CARD: Verdict.BLOCK,      # Block credit cards
        PIIEntityType.US_SSN: Verdict.BLOCK,           # Block SSNs
        PIIEntityType.EMAIL_ADDRESS: Verdict.CONFIRM,  # Ask user about emails
        PIIEntityType.PHONE_NUMBER: Verdict.LOG,       # Just log phone numbers
        # PIIEntityType.IP_ADDRESS not listed = not checked
    }
)
```

**Available PII Entity Types:**

```python
# Pattern-based (work without NLP)
PIIEntityType.EMAIL_ADDRESS
PIIEntityType.CREDIT_CARD
PIIEntityType.CRYPTO
PIIEntityType.IP_ADDRESS
PIIEntityType.PHONE_NUMBER
PIIEntityType.US_SSN
PIIEntityType.US_BANK_NUMBER
PIIEntityType.US_DRIVER_LICENSE
PIIEntityType.US_PASSPORT
PIIEntityType.IBAN_CODE

# NLP-based (require spacy/transformers)
PIIEntityType.PERSON
PIIEntityType.LOCATION
PIIEntityType.DATE_TIME
```

### Selective Checking

Control which contexts are checked:

```python
policy = PIIPolicy(
    entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK},
    check_prompts=True,          # Check user prompts
    check_tool_args=True,        # Check tool arguments
    check_responses=True,        # Check model responses
    check_tool_responses=True,   # Check tool responses
)

# Example: Only check outgoing data (responses)
policy = PIIPolicy(
    entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK},
    check_prompts=False,
    check_tool_args=False,
    check_responses=True,        # Check model output
    check_tool_responses=True,   # Check tool output
)
```

### Detection Threshold

Control sensitivity with `score_threshold` (0.0-1.0):

```python
policy = PIIPolicy(
    entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK},
    score_threshold=0.5  # Default: 0.5 (balanced)
)

# Strict (fewer false positives, may miss some)
policy = PIIPolicy(
    entity_decisions={PIIEntityType.PERSON: Verdict.BLOCK},
    score_threshold=0.8,
    nlp_engine_config={...}
)

# Lenient (catches more, more false positives)
policy = PIIPolicy(
    entity_decisions={PIIEntityType.PERSON: Verdict.BLOCK},
    score_threshold=0.3,
    nlp_engine_config={...}
)
```

## NLP Engine Configuration

### spaCy

```python
policy = PIIPolicy(
    entity_decisions={PIIEntityType.PERSON: Verdict.BLOCK},
    nlp_engine_config={
        "nlp_engine_name": "spacy",
        "models": [{"lang_code": "en", "model_name": "en_core_web_sm"}]
    }
)
```

Available models:
- `en_core_web_sm`: Small (12MB), fast
- `en_core_web_md`: Medium (40MB), better accuracy
- `en_core_web_lg`: Large (560MB), best accuracy

### Transformers

```python
policy = PIIPolicy(
    entity_decisions={PIIEntityType.PERSON: Verdict.BLOCK},
    nlp_engine_config={
        "nlp_engine_name": "transformers",
        "models": [{"lang_code": "en", "model_name": "dslim/bert-base-NER"}]
    }
)
```

### Stanza

```python
policy = PIIPolicy(
    entity_decisions={PIIEntityType.PERSON: Verdict.BLOCK},
    nlp_engine_config={
        "nlp_engine_name": "stanza",
        "models": [{"lang_code": "en", "model_name": "en"}]
    }
)
```

## PII Entity Types

### Pattern-Based (No NLP Required)

These work with the default lightweight configuration:

| Entity Type | Description | Example |
|------------|-------------|---------|
| `EMAIL_ADDRESS` | Email addresses | `user@example.com` |
| `CREDIT_CARD` | Credit card numbers | `4532-1234-5678-9010` |
| `CRYPTO` | Cryptocurrency addresses | `1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa` |
| `IP_ADDRESS` | IPv4/IPv6 addresses | `192.168.1.1` |
| `PHONE_NUMBER` | Phone numbers | `555-123-4567` |
| `US_SSN` | US Social Security Numbers | `123-45-6789` |
| `US_BANK_NUMBER` | US bank account numbers | Various formats |
| `US_DRIVER_LICENSE` | US driver's licenses | State-specific |
| `US_PASSPORT` | US passport numbers | Various formats |
| `IBAN_CODE` | International bank accounts | `GB82WEST12345698765432` |

### NLP-Based (Requires NLP Engine)

These require an NLP engine (spacy/transformers/stanza):

| Entity Type | Description | Example |
|------------|-------------|---------|
| `PERSON` | Person names | `John Smith` |
| `LOCATION` | Locations (if using NER) | `New York` |
| `DATE_TIME` | Dates/times (if using NER) | `January 1, 2024` |

## Integration Examples

### With ADK (Agent Development Kit)

```python
from google.adk import Agent
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

# Create agent with security
agent = Agent(
    name="customer_service",
    instructions="Help customers with their inquiries",
    security_manager=security_manager
)
```

### With CAPSEM Proxy

The proxy automatically applies PII policies from configuration:

```python
# capsem_proxy/config.py
from capsem.policies.pii_policy import PIIPolicy, PIIEntityType
from capsem.models import Verdict

SECURITY_POLICIES = [
    PIIPolicy(
        entity_decisions={
            PIIEntityType.CREDIT_CARD: Verdict.BLOCK,
            PIIEntityType.US_SSN: Verdict.BLOCK,
            PIIEntityType.EMAIL_ADDRESS: Verdict.LOG,
        }
    )
]
```

## Use Cases

### Compliance & Privacy

```python
# GDPR compliance: Block all PII in EU region
from capsem.policies.pii_policy import PIIPolicy, PIIEntityType, DEFAULT_PII_ENTITIES
from capsem.models import Verdict

gdpr_policy = PIIPolicy(
    entity_decisions={entity: Verdict.BLOCK for entity in DEFAULT_PII_ENTITIES},
    check_prompts=True,
    check_responses=True,
)
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
```

## Decision Flow

When PII is detected, the policy returns a `Decision` object:

```python
# Decision attributes
decision.verdict       # BLOCK, CONFIRM, or LOG
decision.reason        # SENSITIVE_DATA (input) or LEAKAGE (output)
decision.details       # "PII detected in prompt: EMAIL_ADDRESS(count=1, score=1.00, action=BLOCK)"
decision.policy        # "PIIDetection"
decision.callback      # "on_model_call", "on_tool_response", etc.
```

The SecurityManager uses "strictest wins" logic:
- Multiple policies: Most restrictive verdict wins
- Multiple PII types: BLOCK > CONFIRM > LOG

## Performance

| Configuration | Latency | Memory | Accuracy |
|--------------|---------|--------|----------|
| Default (spacy lg) | ~15ms | ~600MB | Excellent (context + NER) |
| spacy (sm) | ~10ms | ~100MB | Good (faster, less accurate) |
| transformers | ~50ms | ~600MB | Best (slowest) |

## Troubleshooting

### "PIIPolicy requires presidio-analyzer"

```bash
uv add --group pii presidio-analyzer
```

### "Can't find model 'en_core_web_lg'"

The spacy model isn't downloaded. Install it:
```bash
python -m spacy download en_core_web_lg
```

Or use a smaller model:
```bash
python -m spacy download en_core_web_sm
```

Then configure it:
```python
from capsem.policies.pii_policy import PIIPolicy, PIIEntityType
from capsem.models import Verdict

policy = PIIPolicy(
    entity_decisions={PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK},
    nlp_engine_config={
        "nlp_engine_name": "spacy",
        "models": [{"lang_code": "en", "model_name": "en_core_web_sm"}]
    }
)
```

### "Failed to initialize custom NLP engine"

If using transformers:
```bash
pip install transformers torch
```

If using stanza:
```bash
pip install stanza
```

### Too many false positives

Increase the score threshold:
```python
from capsem.policies.pii_policy import PIIPolicy, PIIEntityType
from capsem.models import Verdict

policy = PIIPolicy(
    entity_decisions={PIIEntityType.PERSON: Verdict.BLOCK},
    score_threshold=0.7  # Higher = fewer false positives
)
```

### Want faster performance?

Use the smaller spacy model:
```bash
python -m spacy download en_core_web_sm
```

Then configure:
```python
from capsem.policies.pii_policy import PIIPolicy, PIIEntityType
from capsem.models import Verdict

policy = PIIPolicy(
    entity_decisions={PIIEntityType.PERSON: Verdict.BLOCK},
    nlp_engine_config={
        "nlp_engine_name": "spacy",
        "models": [{"lang_code": "en", "model_name": "en_core_web_sm"}]
    }
)
```

## Testing

```bash
# Run PII policy tests
uv run pytest capsem/policies/pii_policy_test.py -v

# With coverage
uv run pytest capsem/policies/pii_policy_test.py --cov=capsem.policies.pii_policy
```

## References

- [Microsoft Presidio Documentation](https://microsoft.github.io/presidio/)
- [Presidio Supported Entities](https://microsoft.github.io/presidio/supported_entities/)
- [Presidio Analyzer Architecture](https://microsoft.github.io/presidio/analyzer/)
