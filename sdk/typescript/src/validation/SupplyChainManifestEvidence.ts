// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SupplyChainManifestEvidence} from "../models/SupplyChainManifestEvidence.js";

export const SupplyChainManifestEvidenceSchema: z.ZodType<SupplyChainManifestEvidence> = z.object({
  "blake3": z.string().nullable().exactOptional(),
  "origin": z.string().nullable().exactOptional(),
  "path": z.string(),
  "source": z.string().nullable().exactOptional(),
});
