// Generated from Capsem OpenAPI. Do not edit.

import type { CostCoverage } from "./CostCoverage.js";

export interface ModelUsage {
  "call_count": number;
  "cost_coverage"?: null | CostCoverage;
  "duration_ms": number;
  "estimated_cost_usd": number;
  "input_tokens": number;
  "model": string;
  "output_tokens": number;
  "provider": string;
}
