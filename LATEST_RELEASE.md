version: 1.5.1783863607
---
### Fixed
- Kept the post-deploy binary verifier anchored to the public stable installer
  while validating the selected stable or nightly package manifest, so nightly
  releases prove stable-to-nightly-to-stable switching instead of treating the
  nightly manifest as the initial stable origin.
