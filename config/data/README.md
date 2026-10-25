# Config Data

`credential-broker-policy.json` owns broker capacity, input budgets, finite
enrollment/grant/retirement timeouts, and terminal metadata retention. The
credential owner embeds and validates this closed shape; it supplies no OAuth
registration, vendor identity, user preference or consumer qualification.
Durations are seconds. The configured terminal retry window is 24 hours.
Clients must use a fresh request UUID for new work outside that window.
Maintenance retains live connections, unexpired grants and pending jobs; it
reclaims terminal metadata only after the retry window, once local/provider
retirement has completed. Inventory polls perform no retention I/O.

`genai-prices.json` is Capsem's compact bundled model pricing ledger used by
runtime cost estimation.

Source:

- Repository: https://github.com/pydantic/genai-prices
- File: `prices/data.json`
- Verified upstream commit: `3c7d3751f8c80dbcaf6e9fdb2a7459fd5d882f7e`
  (2026-10-23), 131 supported-provider model entries.
- Raw source SHA-256: `f41075aab069a7dc378f83b7586523f72682a6c9860b23a89ad0b57c073aec41`.
- Pinned raw URL:
  https://raw.githubusercontent.com/pydantic/genai-prices/3c7d3751f8c80dbcaf6e9fdb2a7459fd5d882f7e/prices/data.json

The committed file is not the raw upstream blob. Transform a verified upstream
file through
`build_system/scripts/build/update_genai_prices.py`. The runtime ledger keeps only Capsem's
first-party provider pricing blocks (`anthropic`, `google`, `openai`) and the
fields used by the runtime (`id`, `match`, `context_window`, `prices`). Model
lookup follows upstream matching, UTC date constraints and full-call prompt
size tiers. Capsem does not fuzzy-price unknown model names.

`genai-prices-reviewed.json` records primary-source provenance and exact missing
model IDs not yet published upstream. Refresh merges these entries only while
their exact ID is absent upstream; upstream takes ownership when it adds the ID.
Its single reviewed entry is Google's published standard Gemini 4 Argon API
rates, including the announced December 1 change. The internal observed
`gemini-4-argon-medium` and `gemini-4-argon-trial` IDs are explicit reviewed
aliases of that canonical entry, sharing its rates and date constraints.
Estimates express public API token value, not a subscription invoice. The
alias review does not infer subscription billing equivalence. Existing ledger
rows are not repriced.

Refresh with:

```sh
uv run --project build_system --frozen python \
  build_system/scripts/ci/run-bounded-command.py --timeout-seconds 60 -- \
  uv run --project build_system --frozen python \
  build_system/scripts/build/update_genai_prices.py <verified-upstream.json> \
  config/data/genai-prices.json
```

Fetch the chosen immutable source separately, review upstream shape and rates,
then update the source pin above with the resulting change. There is no current
`just update-prices` recipe.
