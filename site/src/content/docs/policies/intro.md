---
title: Available Security Policies
description: List of available CAPSEM security policies.
---

CAPSEM provides security policies that monitor different aspects of LLM interactions. Each policy can be configured via TOML files.

## Available Policies

| Policy | Description | Use Case |
|--------|-------------|----------|
| [Debug Policy](/policies/debug) | Blocks requests containing `capsem_block` keyword | Development and testing |
| [PII Detection](/policies/pii-detection) | Detects and blocks personally identifiable information using Microsoft Presidio | Prevent sensitive data leaks |

## Next Steps

- [Configuring Policies](/getting-started/configuring-policies) - Learn how to configure and enable policies
- [PII Detection Policy](/policies/pii-detection) - Detailed PII policy documentation
