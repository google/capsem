# Onboarding Milestone

First-launch experience for new Capsem users. Depends on the distribution foundation (asset manager, thin DMG, update story) being complete.

## Scope

### 1. Welcome Screen
- What Capsem is and what it does
- System requirements check: macOS 13+, Apple Silicon, 4 GB RAM
- Check Virtualization.framework entitlement (catch unsigned binary early)

### 2. Credential Entry
- API keys for AI providers (Anthropic, Google, OpenAI)
- Stored in macOS Keychain (per overall plan M8)
- AI gateway injects keys at runtime -- keys never enter the VM
- Validation: test API key with a lightweight call before saving

### 3. MCP Configuration
- Which MCP servers to enable
- Local vs remote classification
- Credential entry for remote tools (GitHub token, Slack token, etc.)
- Per-server allow/deny toggle

### 4. Asset Download
- Progress UI for rootfs.squashfs download
- Uses `DownloadProgress.svelte` component built in the distribution plan
- Disk space check before starting
- Retry on failure with clear error message

### 5. First Boot
- Boot VM with downloaded assets
- Run capsem-doctor subset (sandbox integrity, network isolation)
- Show success/failure state clearly
- On success: transition to terminal view

### 6. Settings Overview
- Quick tour of network policy
- Allowed/blocked domains
- Where to find configuration (user.toml)

## UI Flow

```
Welcome -> Credentials -> MCP Setup -> Asset Download -> First Boot -> Done
```

Each step has a back button. The wizard can be re-entered from settings.

## Dependencies

- Asset manager (distribution plan Phase 2) -- download infrastructure
- DownloadProgress component (distribution plan Phase 3) -- progress UI
- Keychain integration (overall plan M8) -- credential storage
- MCP server manager (existing) -- server configuration

## Implementation Notes

- Wizard state stored in `~/.capsem/onboarding.json` (tracks completed steps)
- WizardView.svelte already exists as a placeholder -- implement the steps there
- Skip wizard on subsequent launches (check onboarding.json)
- `capsem --setup` CLI flag to re-run the wizard
