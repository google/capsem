# Reviewed content detector inputs

These are two independent native plugins, not subprocess wrappers around Python,
Go, browser extensions, NLP models, or credential-validation services. Detector
data is compiled into the binary; requests never download rules.

Reviewed on 2026-10-23 after shallow clone and `git pull --ff-only`:

| Source | Pinned commit | Use | License |
| --- | --- | --- | --- |
| https://github.com/data-privacy-stack/presidio | `4f0e236e31a57767958a3eec2de3fc58dffc3034` | Email pattern and separated SSN pattern; card/SSN validation design | MIT |
| https://github.com/gitleaks/gitleaks | `b58d3f102cf3a2c84cb7f923d05c25c9b1aed84b` | Reviewed nine-rule subset, capture groups and entropy thresholds | MIT |
| https://github.com/momenbasel/keyFinder | `4a3ff3c95776e47cfffae57bd2d40c255afd3210` | Coverage comparison of `js/patterns.js`; no implementation copied | MIT |
| https://github.com/trufflesecurity/trufflehog | `d6ea4a52e3f4705989210e60e9f0da0deafe84bf` | Design comparison only; no AGPL implementation copied | AGPL-3.0 |

Presidio inputs are under
`presidio-analyzer/presidio_analyzer/predefined_recognizers/`:
`generic/email_recognizer.py`, `generic/credit_card_recognizer.py`,
`country_specific/us/us_ssn_recognizer.py`. Phone detection in Presidio uses
`phonenumbers`, not a portable regex. Our deliberately narrower phone recognizer
accepts separated North American numbers; it does not claim global validation.
Card detection removes the Python lookahead (unsupported by linear-time Rust
regex) and applies a 13–19-digit Luhn check. SSN validation rejects invalid areas,
zero groups, mixed separators and published placeholder numbers. No NLP/person,
street address, image/OCR or comprehensive regional PII coverage is claimed.

Gitleaks source is `config/gitleaks.toml`: Anthropic API key, AWS access token,
GitHub app/OAuth/classic/fine-grained tokens, OpenAI key, Slack bot token and
private key. Preserve each selected rule's entropy threshold. First capture
group is the secret when present; PII recognizers instead require whole-match
spans (SSN capture group one is only the area). AWS `EXAMPLE` keys remain excluded.
This is intentionally not the complete Gitleaks rule set. Opaque Capsem broker
references are also recognized; arbitrary historical raw secrets with no known
format cannot be enumerated through the proxy's credential capability.

## Enforcement and limits

Both plugins are disabled unless explicitly configured. PII `content` options
select inbound/outbound/both and email/phone/card/SSN. Secret exfiltration is
outbound only; the existing broker recognizes permitted credential headers at
the matching provider destination. Body/custom-header copies are not permitted
credential use. This does not prove resistance to arbitrary encoding, encryption,
fragmentation across separate HTTP messages or covert channels.

An enabled HTTP content inspection uses the existing semantic body rail: full
body collection before forwarding, 16 MiB wire limit, 32 MiB gzip expansion limit,
60-second deadline and the shared two-body admission budget. It therefore buffers
SSE rather than promising streaming token latency. HTTP transfer chunks are
reassembled before matching. Identity and gzip are supported; other content
encodings, invalid UTF-8 and upgraded tunnels fail closed. Binary uploads and
downloads are outside this text-only filtering scope. Disabled plugins retain
the normal forwarding path.

Recognized MCP-over-HTTP payloads are also refused when PII/secret filtering is
enabled: MCP owns separate argument/result previews, and this initial HTTP-only
implementation cannot safely claim to sanitize that duplicate materialization.
Framed/direct MCP, file operations and cross-message reconstruction are outside
this plugin's scope. Unsupported paths are refused, not silently passed through.

The separate `utf8_guard` validates text media only, preserving non-ASCII Unicode
and binary media. Full bounded collection validates codepoints split across HTTP
chunks and rejects unfinished final sequences before delivery; it does not strip
valid bidirectional/control codepoints or provide an ASCII-only policy.

Masking is byte-length preserving. An all-digit match becomes repeated `1` so an
unquoted JSON number remains syntactically valid; other matches become `*`.
Private-key blocks and other arbitrary structured values should use Block;
raw-text masking cannot promise semantic validity for every embedded protocol.

The audit records detector/plugin identity, outcome and execution timing, never
the matched secret. When content matches, captured/derived body text and headers
are omitted from the ledger while counters and usage are retained. This avoids
rescanning/copying a large sensitive body merely to build a redacted preview.
The actual pre-forward decision is carried to telemetry; it is not rediscovered
from a truncated, mutated or empty preview.
