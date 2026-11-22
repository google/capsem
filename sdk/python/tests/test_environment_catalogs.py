"""Published catalogs bind SDK details to the source that was listed."""
from __future__ import annotations

import asyncio

import pytest
from capsem import Hypervisor, models

from .test_images import gateway


def test_catalog_skill_methods_bind_source_version_and_digest():
    async def run():
        catalog = {"reference": "registry.example/skills:stable", "digest": "sha256:" + "0" * 64,
                   "snapshot": "listed-snapshot"}
        selection = {"id": "review", "version": "1.0.0+preview", "catalog_reference": catalog["reference"],
                     "resolved_digest": catalog["digest"]}
        metadata = {"id": "review", "version": selection["version"], "name": "Review", "publisher": "Fixture",
                    "provenance": "Fixture catalog", "license": "Apache-2.0",
                    "compatibility": {"min_runtime_contract": 1}, "status": "active"}
        files = {"SKILL.md": "review fixture\n", "references/notes.md": "notes\n"}
        response = {"catalog": catalog, "skills": [metadata], "next_cursor": "next"}
        async with gateway(response=response) as (url, received), Hypervisor(url, "gateway-token") as hv:
            page = await hv.skills.list(search="review", after="page", limit=1)
            assert page.next_cursor == "next"
            assert received[-1][1] == "/skills/list?search=review&after=page&limit=1"
            response.clear()
            response.update(selection=selection, metadata=metadata, files=files)
            assert (await hv.skills.get("review", page.skills[0].version, page.catalog)).files == files
            assert received[-1][1] == "/skills/review/info?version=1.0.0%2Bpreview&snapshot=listed-snapshot"
            for record, key, wrong in (
                (selection, "id", "other"), (selection, "version", "1"),
                (metadata, "id", "other"), (metadata, "version", "1"),
                (selection, "catalog_reference", "registry.example/other:stable"),
                (selection, "resolved_digest", "sha256:" + "1" * 64),
            ):
                original = record[key]
                record[key] = wrong
                with pytest.raises(ValueError, match="catalog entry identity changed"):
                    await hv.skills.get("review", "1.0.0+preview", page.catalog)
                record[key] = original
    asyncio.run(run())


def test_global_rules_return_typed_definitions_and_bind_catalog_identity():
    async def run():
        catalog = {"reference": "registry.example/rules:stable", "digest": "sha256:" + "0" * 64,
                   "snapshot": "listed-snapshot"}
        selection = {"id": "review", "version": "1.0.0", "catalog_reference": catalog["reference"],
                     "resolved_digest": catalog["digest"]}
        metadata = {"id": "review", "version": "1.0.0", "name": "Review", "publisher": "Fixture",
                    "provenance": "Fixture catalog", "license": "Apache-2.0",
                    "compatibility": {"min_runtime_contract": 1}, "status": "active"}
        definition = {"kind": "detection", "rule": {
            "name": "Review", "match": "http.host == {{host}}", "level": "high",
        }}
        parameters = {"host": {"type": "text", "default": "api.example", "max_bytes": 70}}
        response = {"catalog": catalog, "rules": [{"metadata": metadata, "kind": "detection"}]}
        async with gateway(response=response) as (url, received), Hypervisor(url, "gateway-token") as hv:
            page = await hv.rules.list(search="review", after="first", limit=12)
            assert page.rules[0].kind == models.CatalogRuleKind.DETECTION
            assert received[-1][1] == "/rules/list?search=review&after=first&limit=12"
            response.clear()
            response.update(selection=selection, metadata=metadata, definition=definition, parameters=parameters)
            detail = await hv.rules.get("review", "1.0.0", page.catalog)
            assert isinstance(detail.definition, models.RuleCatalogDefinitionDetection)
            assert detail.definition.rule.match == "http.host == {{host}}"
            assert isinstance(detail.parameters["host"], models.CatalogRuleParameterText)
            assert detail.parameters["host"].default == "api.example"
            assert received[-1][1] == "/rules/review/info?version=1.0.0&snapshot=listed-snapshot"
            selection["catalog_reference"] = "registry.example/other:stable"
            with pytest.raises(ValueError, match="catalog entry identity changed"):
                await hv.rules.get("review", "1.0.0", page.catalog)
    asyncio.run(run())


def test_global_mcps_return_published_requirements_and_bind_catalog_identity():
    async def run():
        digest = "sha256:" + "0" * 64
        catalog = {"reference": f"registry.example/mcp@{digest}", "digest": digest, "snapshot": "listed-snapshot"}
        selection = {"id": "review", "version": "1.0.0", "catalog_reference": catalog["reference"],
                     "resolved_digest": digest}
        metadata = {"id": "review", "version": "1.0.0", "name": "Review", "publisher": "Fixture",
                    "provenance": "Fixture catalog", "license": "Apache-2.0",
                    "compatibility": {"min_runtime_contract": 1}, "status": "active"}
        response = {"catalog": catalog, "mcps": [{"metadata": metadata, "kind": "remote"}]}
        async with gateway(response=response) as (url, received), Hypervisor(url, "gateway-token") as hv:
            page = await hv.mcps.list(search="review", limit=1)
            assert page.mcps[0].kind == models.McpCatalogKind.REMOTE
            assert received[-1][1] == "/mcps/list?search=review&limit=1"
            response.clear()
            response.update(selection=selection, metadata=metadata, definition={
                "kind": "remote", "url": "https://example.invalid/mcp",
                "secret_binding": {"description": "Workspace key", "required": True},
            }, published_tools=[{"name": "review_file"}])
            detail = await hv.mcps.get("review", "1.0.0", page.catalog)
            assert isinstance(detail.definition, models.McpCatalogDefinitionRemote)
            assert detail.definition.secret_binding.required is True
            assert detail.published_tools[0].name == "review_file"
            assert received[-1][1] == "/mcps/review/info?version=1.0.0&snapshot=listed-snapshot"
            selection["resolved_digest"] = "sha256:" + "1" * 64
            with pytest.raises(ValueError, match="catalog entry identity changed"):
                await hv.mcps.get("review", "1.0.0", page.catalog)
    asyncio.run(run())
