version: 1.5.1784663414
---
### Fixed
- Honored virtqueue interrupt suppression in the KVM VirtioFS worker and
  stopped raising interrupts for empty post-resume queue notifications,
  preventing lost completion wakeups during restored workspace bursts.
