// Generated from Capsem OpenAPI. Do not edit.

import type { SupplyChainChannelEvidence } from "./SupplyChainChannelEvidence.js";
import type { SupplyChainManifestEvidence } from "./SupplyChainManifestEvidence.js";
import type { SupplyChainReference } from "./SupplyChainReference.js";

export interface SupplyChainEvidence {
  "attestations": Array<SupplyChainReference>;
  "channel_index": SupplyChainChannelEvidence;
  "host_sbom": SupplyChainReference;
  "manifest": SupplyChainManifestEvidence;
  "vm_obom": SupplyChainReference;
}
