# Release Architecture

The public release graph is rooted at `release.capsem.org`.

The only mutable manifest URL for a channel is:

```text
/assets/<channel>/manifest.json
```

`channels.json` lists channels and manifest records. A manifest record's
`version` is the manifest contract version, such as `1.0.2`. It is not the
Capsem package version or the runtime revision.

The selected manifest owns two branches:

```text
channel -> packages -> binaries
channel -> runtime -> architecture -> software/images/evidence
```

Packages are delivery containers. Binaries are executable files owned by a
package and carry their own SHA-256, BLAKE3, installed path, version, and SBOM
component reference.

The runtime owns `min_capsem_version`/`max_capsem_version`, software
inventory, the kernel/initrd/rootfs images, and OBOM evidence. It never selects
the current Capsem binary and publishes no config file. Applications are OCI
images resolved through the image catalog, not part of the manifest.

The full release output contract lives in
`web/docs/src/content/docs/architecture/release-output.md`.
