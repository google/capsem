// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SupplyChainChannelEvidence} from "../models/SupplyChainChannelEvidence.js";

export const SupplyChainChannelEvidenceSchema: z.ZodType<SupplyChainChannelEvidence> = z.object({
  "sha256": z.string().nullable().exactOptional(),
  "url": z.string().nullable().exactOptional(),
});
