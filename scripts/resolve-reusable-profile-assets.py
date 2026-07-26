#!/usr/bin/env python3
"""Find one prior GitHub run with an exact reusable profile asset cohort."""

from __future__ import annotations

import argparse
from collections.abc import Callable
from io import BytesIO
import json
import os
from pathlib import Path
import sys
from typing import Any
from urllib.parse import quote, urlparse
from urllib.request import HTTPRedirectHandler, Request, build_opener
from zipfile import BadZipFile, ZipFile


API_ROOT = "https://api.github.com"
USER_AGENT = "capsem-profile-asset-reuse/1"
REQUIRED_ARTIFACTS = (
    "profile-release-selection",
    "vm-assets-arm64",
    "vm-assets-x86_64",
)
SelectionIdentity = tuple[str, str, str, str]


class SafeArtifactRedirectHandler(HTTPRedirectHandler):
    """Keep the GitHub token on API redirects only, never on blob storage."""

    def redirect_request(
        self,
        request: Request,
        file_pointer: object,
        code: int,
        message: str,
        headers: object,
        new_url: str,
    ) -> Request | None:
        parsed = urlparse(new_url)
        if parsed.scheme != "https" or not parsed.netloc:
            raise ValueError(f"GitHub artifact redirect is not HTTPS: {new_url}")
        redirected = super().redirect_request(
            request,
            file_pointer,
            code,
            message,
            headers,
            new_url,
        )
        if redirected is not None and urlparse(request.full_url).netloc != parsed.netloc:
            redirected.remove_header("Authorization")
        return redirected


ARTIFACT_OPENER = build_opener(SafeArtifactRedirectHandler())


def _safe_component(value: object, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{label} is missing")
    if not all(character.isalnum() or character in "._-" for character in value):
        raise ValueError(f"{label} is unsafe: {value!r}")
    return value


def selection_identity(document: object) -> SelectionIdentity:
    if not isinstance(document, dict):
        raise ValueError("profile release selection must be a JSON object")
    if document.get("schema") != "capsem.admin.release_validate.v1":
        raise ValueError("profile release selection has an unsupported schema")
    if document.get("ok") is not True:
        raise ValueError("profile release selection did not validate")
    channel = _safe_component(document.get("channel"), "selection channel")
    profile = _safe_component(document.get("profile"), "selection profile")
    revision = _safe_component(
        document.get("profile_revision"),
        "selection profile revision",
    )
    publication_identity = _safe_component(
        document.get("publication_identity"),
        "selection publication identity",
    )
    expected_publication = f"profile-{channel}-{profile}-{revision}"
    if publication_identity != expected_publication:
        raise ValueError(
            "profile release selection publication identity does not match "
            f"{channel}/{profile}/{revision}"
        )
    return channel, profile, revision, publication_identity


def _artifact_cohort(
    artifacts: object,
) -> dict[str, dict[str, Any]] | None:
    if not isinstance(artifacts, list):
        raise ValueError("GitHub run artifacts response is not a list")
    by_name: dict[str, list[dict[str, Any]]] = {name: [] for name in REQUIRED_ARTIFACTS}
    for row in artifacts:
        if not isinstance(row, dict):
            continue
        name = row.get("name")
        if name in by_name:
            by_name[name].append(row)
    if any(len(rows) != 1 for rows in by_name.values()):
        return None
    cohort = {name: rows[0] for name, rows in by_name.items()}
    for row in cohort.values():
        if row.get("expired") is not False:
            return None
        if not isinstance(row.get("id"), int):
            return None
        url = row.get("archive_download_url")
        if not isinstance(url, str):
            return None
        parsed_url = urlparse(url)
        if parsed_url.scheme != "https" or parsed_url.netloc != "api.github.com":
            return None
    return cohort


def select_reusable_run(
    *,
    runs: object,
    current_run_id: int,
    expected_selection: object,
    artifact_loader: Callable[[int], object],
    selection_loader: Callable[[dict[str, Any]], object],
) -> int | None:
    expected_identity = selection_identity(expected_selection)
    if not isinstance(runs, list):
        raise ValueError("GitHub workflow runs response is not a list")
    for run in runs:
        if not isinstance(run, dict):
            continue
        run_id = run.get("id")
        if (
            not isinstance(run_id, int)
            or run_id == current_run_id
            or run.get("status") != "completed"
        ):
            continue
        cohort = _artifact_cohort(artifact_loader(run_id))
        if cohort is None:
            continue
        try:
            candidate_identity = selection_identity(
                selection_loader(cohort["profile-release-selection"])
            )
        except ValueError:
            continue
        if candidate_identity == expected_identity:
            return run_id
    return None


def _request_bytes(url: str, token: str) -> bytes:
    request = Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "User-Agent": USER_AGENT,
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    with ARTIFACT_OPENER.open(request, timeout=60) as response:
        return response.read()


def _request_json(url: str, token: str) -> dict[str, Any]:
    try:
        document = json.loads(_request_bytes(url, token))
    except json.JSONDecodeError as error:
        raise ValueError(f"GitHub API returned invalid JSON for {url}") from error
    if not isinstance(document, dict):
        raise ValueError(f"GitHub API returned a non-object for {url}")
    return document


def _workflow_runs(repository: str, workflow: str, token: str) -> list[dict[str, Any]]:
    runs: list[dict[str, Any]] = []
    encoded_workflow = quote(workflow, safe="")
    for page in range(1, 11):
        url = (
            f"{API_ROOT}/repos/{repository}/actions/workflows/{encoded_workflow}/runs"
            f"?event=workflow_dispatch&per_page=100&page={page}"
        )
        document = _request_json(url, token)
        rows = document.get("workflow_runs")
        if not isinstance(rows, list):
            raise ValueError("GitHub workflow runs response lacks workflow_runs")
        runs.extend(row for row in rows if isinstance(row, dict))
        if len(rows) < 100:
            break
    return runs


def _run_artifacts(repository: str, run_id: int, token: str) -> list[dict[str, Any]]:
    artifacts: list[dict[str, Any]] = []
    for page in range(1, 11):
        url = (
            f"{API_ROOT}/repos/{repository}/actions/runs/{run_id}/artifacts"
            f"?per_page=100&page={page}"
        )
        document = _request_json(url, token)
        rows = document.get("artifacts")
        if not isinstance(rows, list):
            raise ValueError(f"GitHub run {run_id} response lacks artifacts")
        artifacts.extend(row for row in rows if isinstance(row, dict))
        if len(rows) < 100:
            break
    return artifacts


def _selection_from_artifact(artifact: dict[str, Any], token: str) -> dict[str, Any]:
    url = artifact["archive_download_url"]
    try:
        with ZipFile(BytesIO(_request_bytes(url, token))) as archive:
            members = [
                member
                for member in archive.infolist()
                if not member.is_dir()
                and Path(member.filename).name == "profile-release-selection.json"
            ]
            if len(members) != 1:
                raise ValueError(
                    "profile-release-selection artifact must contain exactly one "
                    "profile-release-selection.json"
                )
            document = json.loads(archive.read(members[0]))
    except BadZipFile as error:
        raise ValueError("profile-release-selection artifact is not a ZIP archive") from error
    except json.JSONDecodeError as error:
        raise ValueError("profile-release-selection artifact contains invalid JSON") from error
    if not isinstance(document, dict):
        raise ValueError("profile-release-selection artifact contains a non-object")
    return document


def find_reusable_run(
    *,
    repository: str,
    workflow: str,
    current_run_id: int,
    expected_selection: object,
    token: str,
) -> int | None:
    runs = _workflow_runs(repository, workflow, token)
    return select_reusable_run(
        runs=runs,
        current_run_id=current_run_id,
        expected_selection=expected_selection,
        artifact_loader=lambda run_id: _run_artifacts(repository, run_id, token),
        selection_loader=lambda artifact: _selection_from_artifact(artifact, token),
    )


def _repository(value: str) -> str:
    parts = value.split("/")
    if len(parts) != 2:
        raise argparse.ArgumentTypeError("repository must be OWNER/REPO")
    try:
        return "/".join(_safe_component(component, "repository component") for component in parts)
    except ValueError as error:
        raise argparse.ArgumentTypeError(str(error)) from error


def _workflow(value: str) -> str:
    if Path(value).name != value or not value.endswith(".yaml"):
        raise argparse.ArgumentTypeError("workflow must be one YAML file name")
    try:
        return _safe_component(value, "workflow")
    except ValueError as error:
        raise argparse.ArgumentTypeError(str(error)) from error


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True, type=_repository)
    parser.add_argument("--workflow", required=True, type=_workflow)
    parser.add_argument("--current-run-id", required=True, type=int)
    parser.add_argument("--selection", required=True, type=Path)
    parser.add_argument("--github-output", required=True, type=Path)
    args = parser.parse_args(argv)
    if args.current_run_id <= 0:
        parser.error("--current-run-id must be positive")
    token = os.environ.get("GITHUB_TOKEN")
    if not token:
        print("profile asset reuse resolution failed: GITHUB_TOKEN is missing", file=sys.stderr)
        return 1
    try:
        expected_selection = json.loads(args.selection.read_text(encoding="utf-8"))
        run_id = find_reusable_run(
            repository=args.repository,
            workflow=args.workflow,
            current_run_id=args.current_run_id,
            expected_selection=expected_selection,
            token=token,
        )
        with args.github_output.open("a", encoding="utf-8") as output:
            output.write(f"run_id={run_id or ''}\n")
    except (OSError, ValueError) as error:
        print(f"profile asset reuse resolution failed: {error}", file=sys.stderr)
        return 1
    if run_id is None:
        print("no exact reusable profile asset cohort found; building once")
    else:
        print(f"reusing exact profile asset cohort from GitHub run {run_id}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
