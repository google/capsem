"""Canonical blueprints through the authenticated gateway."""
from __future__ import annotations

from . import _operations as api
from . import models
from ._transport import Transport


class Environments:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def list(
        self, *, search: str | None = None, after: str | None = None,
        limit: int | None = None, request_timeout: float | None = None,
    ) -> models.EnvironmentListResponse:
        """List visible committed environments; follow the cursor to continue search."""
        return await api.list_environments(
            self._transport, search=search, after=after, limit=limit, request_timeout=request_timeout,
        )

    async def create(
        self, request: models.EnvironmentCreateRequest, *, request_timeout: float | None = None,
    ) -> models.EnvironmentMutationResponse:
        """Submit an explicit request ID; approval may precede application."""
        request_id = request.request_id
        result = await api.create_environment(
            self._transport, body=request, request_timeout=request_timeout,
        )
        if result.request_id != request_id:
            raise ValueError("environment request identity changed")
        return result

    async def change(
        self, id: str, request: models.EnvironmentChangeIntent, *, request_timeout: float | None = None,
    ) -> models.EnvironmentMutationResponse:
        """Submit an edit, archive, or exact image update with a caller-chosen request ID."""
        request_id = request.request_id
        result = await api.change_environment(
            self._transport, id=id, body=request, request_timeout=request_timeout,
        )
        if result.request_id != request_id or result.environment_id != id:
            raise ValueError("environment change identity changed")
        return result

    async def info(
        self, id: str, *, request_timeout: float | None = None,
    ) -> models.EnvironmentCurrentResponse:
        """Read the verified current revision and committed provenance."""
        result = await api.get_environment_info(
            self._transport, id=id, request_timeout=request_timeout,
        )
        if result.document.identity.id != id:
            raise ValueError("environment identity changed")
        return result

    async def promote(
        self, id: str, request: models.EnvironmentPromotionRequest, *, request_timeout: float | None = None,
    ) -> models.EnvironmentMutationResponse:
        """Promote an exact fork into its parent for future sandboxes."""
        request_id = request.request_id
        result = await api.promote_environment(
            self._transport, id=id, body=request, request_timeout=request_timeout,
        )
        if result.request_id != request_id or result.environment_id != id:
            raise ValueError("environment promotion identity changed")
        return result

    async def check_image_update(
        self, reference: models.EnvironmentRevisionReference, *, request_timeout: float | None = None,
    ) -> models.EnvironmentImageUpdateResponse:
        result = await api.check_environment_image_update(
            self._transport, id=reference.environment_id, revision_id=reference.revision_id,
            request_timeout=request_timeout,
        )
        if result.environment != reference:
            raise ValueError("environment revision identity changed")
        return result

    async def list_revisions(
        self, id: str, *, before: int | None = None, limit: int | None = None,
        request_timeout: float | None = None,
    ) -> models.EnvironmentRevisionListResponse:
        return await api.list_environment_revisions(
            self._transport, id=id, before=before, limit=limit, request_timeout=request_timeout,
        )

    async def revision(
        self, reference: models.EnvironmentRevisionReference, *, request_timeout: float | None = None,
    ) -> models.EnvironmentRevisionDetailResponse:
        result = await api.get_environment_revision(
            self._transport, id=reference.environment_id, revision_id=reference.revision_id,
            request_timeout=request_timeout,
        )
        if result.summary.environment != reference:
            raise ValueError("environment revision identity changed")
        return result

    async def plugins(
        self, reference: models.EnvironmentRevisionReference | None = None, *,
        request_timeout: float | None = None,
    ) -> models.EnvironmentPluginListResponse:
        """Read plugins against an exact current revision, or defaults for creation."""
        if reference is None:
            result = await api.list_new_environment_plugins(self._transport, request_timeout=request_timeout)
        else:
            result = await api.list_environment_plugins(
                self._transport, id=reference.environment_id, revision_id=reference.revision_id,
                request_timeout=request_timeout,
            )
        if result.revision != reference:
            raise ValueError("environment plugin revision identity changed")
        return result

    async def validate_toml(
        self, base: models.EnvironmentRevisionReference, toml: str, *, request_timeout: float | None = None,
    ) -> models.EnvironmentTomlValidationResponse:
        result = await api.validate_environment_toml(
            self._transport, id=base.environment_id, revision_id=base.revision_id,
            body=models.EnvironmentTomlValidationRequest(toml=toml), request_timeout=request_timeout,
        )
        if result.base != base:
            raise ValueError("environment revision identity changed")
        return result

    async def rules(
        self, reference: models.EnvironmentRevisionReference, *, request_timeout: float | None = None,
    ) -> models.EnvironmentRuleListResponse:
        """Read the rules compiled for an exact current environment revision."""
        result = await api.list_environment_rules(
            self._transport, id=reference.environment_id, revision_id=reference.revision_id,
            request_timeout=request_timeout,
        )
        if result.revision != reference:
            raise ValueError("environment rule revision identity changed")
        return result

    async def skills(
        self, reference: models.EnvironmentRevisionReference, *, search: str | None = None,
        after: str | None = None, limit: int | None = None, request_timeout: float | None = None,
    ) -> models.EnvironmentSkillListResponse:
        """List selected skill pins and their admission status for an exact saved revision."""
        result = await api.list_environment_skills(
            self._transport, id=reference.environment_id, revision_id=reference.revision_id,
            search=search, after=after, limit=limit, request_timeout=request_timeout,
        )
        if result.revision != reference:
            raise ValueError("environment skill revision identity changed")
        return result

    async def skill(
        self, reference: models.EnvironmentRevisionReference, skill_id: str, *, request_timeout: float | None = None,
    ) -> models.EnvironmentSkillDetailResponse:
        """Read the complete UTF-8 dictionary for a selected skill."""
        result = await api.get_environment_skill(
            self._transport, id=reference.environment_id, revision_id=reference.revision_id,
            skill_id=skill_id, request_timeout=request_timeout,
        )
        if (result.revision != reference or result.selection.id != skill_id or result.metadata.id != skill_id
                or result.metadata.version != result.selection.version):
            raise ValueError("environment skill identity changed")
        return result
