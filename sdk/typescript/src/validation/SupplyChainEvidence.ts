// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SupplyChainEvidence} from "../models/SupplyChainEvidence.js";
import {SupplyChainChannelEvidenceSchema} from "./SupplyChainChannelEvidence.js";
import {SupplyChainManifestEvidenceSchema} from "./SupplyChainManifestEvidence.js";
import {SupplyChainReferenceSchema} from "./SupplyChainReference.js";

export const SupplyChainEvidenceSchema: z.ZodType<SupplyChainEvidence> = z.object({
  "attestations": z.array(z.lazy(() => SupplyChainReferenceSchema)),
  "channel_index": z.lazy(() => SupplyChainChannelEvidenceSchema),
  "host_sbom": z.lazy(() => SupplyChainReferenceSchema),
  "manifest": z.lazy(() => SupplyChainManifestEvidenceSchema),
  "vm_obom": z.lazy(() => SupplyChainReferenceSchema),
});
