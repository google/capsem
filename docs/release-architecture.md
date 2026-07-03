# Release Architecture

The public release graph is rooted at `release.capsem.org`.

The only mutable manifest URL for a channel is:

```text
/assets/<channel>/manifest.json
```

`channels.json` lists channels and manifest records. A manifest record's
`version` is the manifest contract version, such as `1.0.2`. It is not the
Capsem package version, VM asset version, profile revision, or profile image
revision.

The selected manifest owns two branches:

```text
channel -> packages -> binaries
channel -> profiles -> architecture -> config/software/images/evidence
```

Packages are delivery containers. Binaries are executable files owned by a
package and carry their own SHA-256, BLAKE3, installed path, version, and SBOM
component reference.

Profiles own `min_capsem_version`, config files, software inventory, profile
images, and ABOM/OBOM evidence. Profiles never select the current Capsem binary.

The full release output contract lives in
`docs/src/content/docs/architecture/release-output.md`.
