# S19a - Marketing Site Refresh

## Status

Not started. Runs after the docs/security architecture sprints have enough
truth to market accurately, and before the final release gate.

Current-site baseline screenshots were captured before implementation at
`artifacts/S19a-marketing-site-refresh/current-ui-baseline/`:

- `01-hero.png`
- `02-features.png`
- `03-security.png`
- `04-how-it-works.png`
- `05-faq.png`

These are baseline review artifacts for the UI that exists today. The sprint's
final visual gate still requires the refreshed hero plus one screenshot for
each of the four target pillars below.

## Goal

Refresh the Capsem landing page so it clearly explains the product we are
building: fast AI workspaces that ship quickly, run safely, scale without drag,
and satisfy enterprise security/operations requirements.

This sprint updates the marketing site, not the product docs. Claims must map
to shipped features, active sprint commitments, or explicitly marked roadmap
items. No vague security theater, no legacy v1 language, and no claims that
contradict the profile/security/event architecture.

## Site Shape

The landing page should be organized around four product pillars:

1. **Ship Fast With AI**
   - one-click idea-to-VM flow;
   - Codex/Claude/agent workbench direction;
   - terminal fallback and SDK-backed sessions;
   - profiles that carry tools, packages, MCP servers, skills, and controls;
   - profile-backed VM create, fork, exec, snapshots, and reusable workflows;
   - paginated timeline/workbench for reviewing AI work.
2. **Ship Safely**
   - VM isolation;
   - release SBOMs, SLSA provenance, signed manifests, and package
     attestations as supply-chain security proof points;
   - profile-owned enforcement packs and detection packs;
   - realtime CEL enforcement with allow/block/ask/rewrite decisions;
   - Sigma-compatible detection over normalized Capsem events, with backtest and
     fast matching over HTTP, DNS, MCP, model, file, process, and timeline
     activity;
   - Security Engine pipeline: preprocessors, enforcement, ask/confirm, detection,
     postprocessors, emitter;
   - file, process, network, DNS, MCP, and model activity lifted into normalized
     events;
   - Sigma forensic analysis of a specific timeline/session using the same
     unified event model as live detection;
   - ask/confirm, quarantine, restore/revert, and auditable decisions.
3. **Scale Your Productivity Without Drag**
   - low boot time and fast execution;
   - many VMs without status/list SQL fan-out;
   - profile-selected lazy asset download and cache reuse;
   - efficient Network/File/Process engines with bounded thread pools;
   - live metrics from in-memory accumulators;
   - fast fork/clone/snapshot workflows and predictable cleanup.
4. **Enterprise Ready**
   - signed profile catalogs and profile lifecycle states;
   - corp profiles, package/tool contracts, profile-owned assets, and VM pins;
   - `capsem-admin` for profile/settings/enforcement/detection/image/manifest
     workflows;
   - OpenTelemetry and VM health: model calls, provider/model summaries, token
     counts, estimated cost, detection findings, enforcement counters,
     forward-plugin health, and activity counters;
   - forensic `session.db`, structured timeline, Sigma hunt over a specific
     timeline/session, support bundles, and audit trails;
   - SOAR/centralized forward-plugin integration direction.

## Content Requirements

- Replace or rewrite any homepage copy that still centers old security levels,
  standalone `[mcp]`, `config/defaults.json`, hand-edited image settings, or
  generic sandbox language.
- Add feature claims for profile-backed VM creation, signed profile catalogs,
  profile-owned packages/assets, standard `mcpServers`, skills, rules, and
  timeline/workbench review.
- Add SBOM and release-attestation proof points under the security story. Keep
  the wording aligned with S07b: host Rust workspace SBOM is shipped/attested;
  profile-derived guest package/tool SBOM remains image-verification work until
  that sprint lands it.
- Add security copy for realtime enforcement and detection as first-class
  features: CEL-backed enforcement for live decisions, Sigma-compatible
  detection with backtest, default diverse evidence samples, fast matching over
  normalized events, and forensic Sigma analysis against a chosen timeline or
  session.
- Add performance copy for the Security Engine only from S08d benchmark
  artifacts: VM-originated allow/block/detect latency, CEL/Sigma match speed,
  rule-count scaling, and backtest/hunt scan rates. No numeric speed claim may
  appear unless it cites or links to the recorded benchmark result page.
- Explain the unified event/timeline story as the reason enforcement, Sigma
  findings, telemetry, audit, and support bundles line up. Use strong language
  around a "bulletproof forensic trail" only when paired with concrete proof
  points: stable event ids, VM/profile/user attribution, resolved-event
  journals, backtest evidence rows, and timeline/session replay.
- Distinguish shipped/near-term capabilities from roadmap items without making
  the page timid. The page can say where Capsem is going, but release claims
  must match the sprint tracker.
- Make the first viewport say what Capsem is: secure AI workspaces/VMs for
  shipping agentic software fast and safely.
- Avoid turning the page into docs. Each section should be crisp marketing copy
  with concrete proof points and links into documentation for details.
- Link the enterprise/security sections to docs pages created in S19:
  enforcement, detection format, profile catalogs, corp deployment,
  `capsem-admin`, VM health/OTel, and timeline/workbench.
- Keep all marketing copy in `site/src/lib/data.ts` unless the existing site
  architecture has changed.
- Capture and save five release-review screenshots:
  1. hero/first viewport;
  2. Ship Fast With AI section;
  3. Ship Safely section;
  4. Scale Your Productivity Without Drag section;
  5. Enterprise Ready section.
  Screenshots must come from the running site, not design mockups.

## Implementation Notes

- Follow the marketing site architecture in `site/`: Astro/Svelte/Tailwind,
  single landing page, copy centralized in `site/src/lib/data.ts`.
- Reuse existing components where possible. Add a new component only if the
  four-pillar story cannot be represented cleanly with the current sections.
- The performance/scaling pillar should feel operational and concrete, not like
  benchmark theater. Use numbers only when measured by S08d or another recorded
  benchmark artifact.
- The enterprise pillar should mention observability, forensic trails, Sigma
  timeline analysis, SOAR/centralized forward plugin, OpenTelemetry, and corp
  profile governance as one coherent operating story.

## Testing Matrix

- Unit/contract: copy/data exports type-check; new section data matches
  component props; links point to existing or S19-created docs routes.
- Functional: marketing site builds successfully and renders all four pillar
  sections.
- Adversarial: copy audit proves no v1/defaults-json/hand-edited-image-settings
  language remains; no unsupported production claims appear as shipped.
- Visual: capture the five required release-review screenshots on desktop and
  at least one mobile viewport. The screenshots must show no text overlap,
  unreadable contrast, or card nesting, and the first viewport must make
  Capsem's product category obvious.
- Performance: landing page build output remains lightweight; no unnecessary
  client hydration for static marketing sections.
- Documentation: linked docs pages exist or are tracked in S19 before release.
- Benchmarks: any CEL/Sigma/security-engine speed claim has a matching S08d
  benchmark artifact and benchmark docs link.

## Done Means

- The landing page has four clear pillars: Ship Fast With AI, Ship Safely,
  Scale Your Productivity Without Drag, and Enterprise Ready.
- All feature claims align with the sprint tracker and release state.
- Security, detection, observability, corp profile, forensic, SOAR/remote
  enforcement/forward-plugin, and OpenTelemetry capabilities are represented
  without overclaiming.
- Site build and responsive visual verification pass, including the hero
  screenshot plus one screenshot for each of the four pillar sections.
