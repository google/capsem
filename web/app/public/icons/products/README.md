# Supported product icons

Bundled, passive images for the six supported catalog entries. The catalog,
Info panel and existing-session rows use these local files through `<img>`;
they never hotlink a vendor, GitHub favicon service or tracking endpoint.
Assets are copied unchanged. No image generator, recoloring or invented logos.

Retrieved and inspected on 2026-10-23. The hashes below pin the retrieved web
artwork for source/provenance review; they are not a runtime hashing mechanism.

| Catalog image | Publisher | Bundled icon | Official source |
| --- | --- | --- | --- |
| Development | Capsem | `capsem.svg` | Repository `web/graphics/tauri/icon.svg`, canonical app artwork |
| Codex CLI | OpenAI | `openai.svg` | [Official OpenAI documentation mark](https://developers.openai.com/favicon.svg) |
| Claude Code | Anthropic | `claude.png` | [Official Claude Code documentation icon](https://code.claude.com/docs/_mintlify/favicons/claude-code/pLsy-mRpNksna2sx/_generated/favicon/android-chrome-192x192.png) |
| Claude Desktop | Anthropic | `claude.png` | Same official Claude product asterisk; shared brand, not a claimed distinct Linux application logo |
| Antigravity CLI | Google | `antigravity.svg` | [Official Antigravity icon](https://antigravity.google/favicon.svg) |
| Antigravity App 2 | Google | `antigravity.svg` | Same official Antigravity product mark |

Codex CLI does not publish a standalone application icon in its official source
tree. Its card identifies OpenAI using OpenAI's own documentation mark; this is
not presented as a newly invented Codex-specific symbol. Claude's official raster is 192×192. Capsem, OpenAI and Antigravity are vectors.
The official homepages advertise the Claude and Antigravity icon sources.

## Rights and terms

- Capsem artwork is first-party source under this repository's Apache-2.0
  license. Its source blob is `daf219968e4374498b02837337a45d362fc3d7e9`.
- OpenAI, Claude/Anthropic and Antigravity/Google names and marks belong to
  their owners. No open-source license for the retrieved website artwork is
  asserted or inferred from a source repository's code license. These copies
  identify the cataloged products; Capsem does not imply vendor endorsement.
- OpenAI's [brand terms](https://openai.com/brand/) remain applicable. That
  page returned HTTP 403 during collection, so no separate grant was verified.
  The official Codex repository is Apache-2.0, whose section 6 excludes a
  general trademark license; it is not the source/license for this SVG.
- The [official Claude Code license](https://github.com/anthropics/claude-code/blob/65823ec0034ed0aa0266bc43903fefd3ff94a327/LICENSE.md)
  states Anthropic retains all rights and links its
  [Commercial Terms](https://www.anthropic.com/legal/commercial-terms).
  That notice does not establish a separate artwork redistribution license.
- Antigravity links its [product terms](https://antigravity.google/terms) and
  [Google terms](https://policies.google.com/terms). Google brand guidance is
  at its [Brand Resource Center](https://about.google/brand-resource-center/).
  No artwork redistribution or trademark grant was found in the CLI source.

Local official-source audits, not asset license grants:
`openai/codex@ea819e1f4d0759777106e5a6fb9984752b20f972`,
`anthropics/claude-code@65823ec0034ed0aa0266bc43903fefd3ff94a327`,
`google-antigravity/antigravity-cli@fd851f1da40e4347f1d05d211dcb45f94aede4c4`.

## Exact file pins (SHA-256)

```text
d4f132075a98785d21a9feaa3adffe0ece2f7be66e09b155147105f5fe42d002  capsem.svg
8e1b976ba47e927ac2928303fd5362fcadfefd68743506510596f107df676e58  openai.svg
d4e5e8e9990ac52f147a74ae80b9e69cdc612264b84a76cdb5c43b53b73a1707  claude.png
de7911c3d206a3d2fe3d5b4e76e84b185c581d7a259406f64db247b5b824f02e  antigravity.svg
```

This is a metadata/asset change, not an image rebuild or publication. A running
fixture must consume the refreshed catalog metadata and a rebuilt frontend
bundle before its cards can show these icons. Dark/light/DPR2 card, Info and
session visual acceptance remains with the UI owner.
