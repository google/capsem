# Sprint: UI Extension Contract Research

## Tasks

- [x] Create research sprint plan.
- [x] Fetch upstream source into ignored `private/`.
- [x] Inspect VS Code extension UI contracts.
- [x] Inspect Obsidian extension UI contracts.
- [x] Write findings and Capsem recommendations.
- [x] Record source versions.

## Notes

- Design constraint: stay close to JavaScript, Chrome extension, VS Code, and
  Obsidian mental models where possible. This helps both human authors and
  agent authors reason correctly.
- Source snapshots:
  - `microsoft/vscode` at `32179cb`.
  - `microsoft/vscode-extension-samples` at `61fda64`.
  - `obsidianmd/obsidian-api` at `2e88986`.
  - `obsidianmd/obsidian-sample-plugin` at `dc2fa22`.
  - `obsidianmd/obsidian-developer-docs` at `96e5dae`.
- VS Code gives us the safer boundary: manifest contribution points, extension
  host registration, webview iframe, CSP, local resource roots, nonce, state,
  and `postMessage`.
- Obsidian gives us the ergonomic side-pane model: `Plugin.onload`,
  `registerView`, `ItemView`, `WorkspaceLeaf`, `ensureSideLeaf`, `setViewState`,
  and `revealLeaf`.
- Capsem should copy VS Code's isolation and Obsidian's workspace/pane feel.
  Do not copy Obsidian's ambient DOM/app authority.
- Findings written in `docs/ui-extension-contract-research.md`.

## Coverage Ledger

- Primary source: VS Code source/API, VS Code samples, Obsidian API/sample/docs.
- Comparative analysis: recorded in `docs/ui-extension-contract-research.md`.
- Prototype impact: recommends manifest `contributes`, structured chat parts,
  MIME-style chat renderers, sandboxed side-panel webviews, `postMessage`,
  `getState`/`setState`, and manifest-gated `fetch`.
- Missing/deferred: no UI implementation in this sprint.
