## Features
- Allow branching when checkpointing a VM, so users can create a new branch from a checkpoint and continue working without affecting the original branch using a different models/prompts

## Security
- Keyless vm -- find a way to avoid injecting SSH keys and AI keys in the image
- FS watch - is VM side should be client side.
- Network Default policy should be by HTTP verb not global (eg allows get, head but no other verbs)
- Add path restriction to the web network policy (eg allow github.com but only /repos/*) or specific repos and organizations.