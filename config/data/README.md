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
- Raw URL:
  https://raw.githubusercontent.com/pydantic/genai-prices/main/prices/data.json

The committed file is not the raw upstream blob. `just update-prices` fetches
the upstream file and transforms it through
`build_system/scripts/build/update_genai_prices.py`. The runtime ledger keeps only Capsem's
first-party provider pricing blocks (`anthropic`, `google`, `openai`) and the
fields used by the runtime (`id`, `match`, `context_window`, `prices`). Model
lookup uses the upstream `match` clauses exactly; Capsem does not fuzzy-price
unknown model names.

Refresh with:

```sh
just update-prices
```
