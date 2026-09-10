"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .model_base import Model
from .profile_existing_vm_update_semantics import ProfileExistingVmUpdateSemantics
from .profile_new_session_update_semantics import ProfileNewSessionUpdateSemantics
from .profile_upgrade_action import ProfileUpgradeAction


class ProfileUpdateSemantics(Model):
    existing_vms: ProfileExistingVmUpdateSemantics
    new_sessions: ProfileNewSessionUpdateSemantics
    upgrade_action: ProfileUpgradeAction
