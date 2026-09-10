"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .model_base import Model
from .supply_chain_channel_evidence import SupplyChainChannelEvidence
from .supply_chain_manifest_evidence import SupplyChainManifestEvidence
from .supply_chain_reference import SupplyChainReference


class SupplyChainEvidence(Model):
    attestations: list[SupplyChainReference]
    channel_index: SupplyChainChannelEvidence
    host_sbom: SupplyChainReference
    manifest: SupplyChainManifestEvidence
    vm_obom: SupplyChainReference
