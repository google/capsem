version: 1.5.1783538942
---
### Fixed
- Made the public installer resolve `.pkg` and `.deb` packages from the
  stable release-channel manifest instead of GitHub's latest release pointer,
  and updated the marketing/docs download links to use the stable channel.
- Made `capsem update --assets` accept split-lane release-channel profile
  manifests, hydrate their exact VM image artifact URLs, and install a
  validated local v2 asset manifest for runtime compatibility.
- Made the tray gateway client report corrupt gateway tokens as ordinary
  request errors instead of panicking its background poller.
