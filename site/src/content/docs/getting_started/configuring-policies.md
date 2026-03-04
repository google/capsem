---
title: Configuring Security Policies
description: How to configure and enable CAPSEM security policies.
---

CAPSEM security policies can be configured in two ways: using TOML configuration files (recommended) or programmatically in Python code.

## Using CAPSEM Package Directly

### Configuration Files

Create a directory with TOML configuration files for your policies:

```
config/
├── debug.toml
├── pii.toml
└── custom.toml
```

### Loading Policies from Configuration

```python
from capsem.config_loader import load_policies_from_directory

# Load all policies from config directory
security_manager = load_policies_from_directory("config/")

# Policies are now ready to use
print(f"Loaded {len(security_manager.policies)} policies")
```

### Policy Configuration Examples

**config/debug.toml:**
```toml
enabled = true
```

**config/pii.toml:**
```toml
enabled = true

check_prompts = true
check_responses = true

[entity_decisions]
EMAIL_ADDRESS = "BLOCK"
CREDIT_CARD = "BLOCK"
```

### Programmatic Configuration

You can also create policies directly in Python:

```python
from capsem.manager import SecurityManager
from capsem.policies.debug_policy import DebugPolicy
from capsem.policies.pii_policy import PIIPolicy, PIIEntityType
from capsem.models import Verdict

# Create security manager
security_manager = SecurityManager()

# Add Debug policy
security_manager.add_policy(DebugPolicy())

# Add PII policy with custom config
pii_policy = PIIPolicy(
    entity_decisions={
        PIIEntityType.EMAIL_ADDRESS: Verdict.BLOCK,
        PIIEntityType.CREDIT_CARD: Verdict.BLOCK,
        PIIEntityType.PERSON: Verdict.LOG
    },
    check_prompts=True,
    check_responses=True
)
security_manager.add_policy(pii_policy)
```

## Using CAPSEM Proxy

When using the CAPSEM proxy, policies are configured via a config directory.

### Configuration Directory

Create a `config/` directory with TOML files:

```
config/
├── debug.toml
├── pii.toml
```

### Starting the Proxy

```bash
# Use default config/ directory
python -m capsem_proxy.run_proxy

# Use custom directory
python -m capsem_proxy.run_proxy --config-dir /path/to/config
```

The proxy displays enabled policies on startup:

```
============================================================
  CAPSEM PROXY - Multi-tenant LLM Security Proxy
============================================================
  Security Policies:
    Config Dir: config
    Enabled Policies: 2
      ✓ Debug
      ✓ PIIDetection
============================================================
```

## Enabling and Disabling Policies

### Via Configuration File

Set `enabled = true` or `enabled = false`:

```toml
# Enable the policy
enabled = true
```

```toml
# Disable the policy
enabled = false
```

### Programmatically

Simply don't add the policy to the SecurityManager:

```python
# Only add policies you want enabled
security_manager = SecurityManager()
security_manager.add_policy(DebugPolicy())  # Only Debug enabled
```

## Policy Actions

Policies can take different actions when they detect issues:

| Action | Behavior |
|--------|----------|
| BLOCK | Request blocked, returns error |
| CONFIRM | Requires user confirmation (future feature) |
| LOG | Logged but allowed to proceed |
| ALLOW | Proceeds without logging |

## Multiple Policies

When multiple policies are enabled:

- **Most restrictive wins**: If any policy blocks, the request is blocked
- **All must pass**: Request only proceeds if all policies allow it

## Next Steps

- [Available Policies](/policies/intro) - See all available policies
- [PII Detection Policy](/policies/pii-detection) - Detailed PII configuration
- [Proxy Setup](/getting-started/proxy) - Running the proxy
