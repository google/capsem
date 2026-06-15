# Config Data

`genai-prices.json` is Capsem's compact bundled model pricing ledger used by
runtime cost estimation.

Source:

- Repository: https://github.com/pydantic/genai-prices
- File: `prices/data.json`
- Raw URL:
  https://raw.githubusercontent.com/pydantic/genai-prices/main/prices/data.json

The committed file is not the raw upstream blob. `just update-prices` fetches
the upstream file and transforms it through
`scripts/update_genai_prices.py`. The runtime ledger keeps only Capsem's
first-party provider pricing blocks (`anthropic`, `google`, `openai`) and the
fields used by the runtime (`id`, `match`, `context_window`, `prices`). Model
lookup uses the upstream `match` clauses exactly; Capsem does not fuzzy-price
unknown model names.

Refresh with:

```sh
just update-prices
```
