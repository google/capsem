"""Citadel guard: a systemd service must not own its package updater.

The release qualification that found this reached the exact candidate, launched
``capsem update --yes``, and then stopped ``capsem.service`` from the Debian
preinstall hook. Because the updater, apt, and dpkg were children of that unit,
systemd killed all of them in the middle of package replacement.

The Rust regression proves command construction. This source guard records why
the ownership boundary exists and fails in the fast phase if it is weakened.
"""

from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
UPDATE_COMMAND = PROJECT_ROOT / "crates/capsem-service/src/update_command.rs"
SERVICE_MAIN = PROJECT_ROOT / "crates/capsem-service/src/main.rs"
SERVICE_INSTALL = PROJECT_ROOT / "crates/capsem/src/service_install.rs"
DEB_PREINSTALL = PROJECT_ROOT / "scripts/deb-preinst.sh"
DEB_POSTINSTALL = PROJECT_ROOT / "scripts/deb-postinst.sh"
SERVICE_OWNERSHIP = (
    PROJECT_ROOT / "build_system/packaging/shared/service-owned-update"
)
INSTALL_COHORT = PROJECT_ROOT / "build_system/packaging/shared/retire-cohort"
REPACK_DEB = PROJECT_ROOT / "scripts/repack-deb.sh"
INSTALLATION_SKILL = PROJECT_ROOT / "skills/dev-installation/SKILL.md"
RELEASE_SKILL = PROJECT_ROOT / "skills/release-process/SKILL.md"
RELEASE_CI_INVARIANTS = (
    PROJECT_ROOT / "skills/release-process/references/ci-invariants.md"
)

SYSTEMD_UPDATE_OWNERSHIP_RATIONALE = """\
Linux package replacement stops capsem.service from the Debian preinstall hook.
An updater launched as a child of that service stays in its cgroup, so stopping
the service kills capsem, apt, dpkg, and the preinstall script halfway through
the transaction.

The complete `capsem update --yes` transaction must run in a fixed, sibling
transient user service. Wrapping only apt leaves the parent unable to activate
profiles or record the audit result. `--pipe` is forbidden because the reader
dies with capsem.service and can SIGPIPE the surviving updater. The fixed unit
name also prevents the restarted service from launching a duplicate update.
`INVOCATION_ID` is inherited, so it must be paired with `SYSTEMD_EXEC_PID`
matching the current process; otherwise children of the GitHub runner, an IDE,
or any unrelated systemd service are misidentified as capsem.service.

The first package carrying that fix is still installed by the previous service,
which cannot run code it does not have. Its dpkg transaction must therefore be
recognized from `/proc/self/cgroup` by the new package preinstall. While that
transaction is inside capsem.service, preinstall must neither stop the unit nor
retire its helper cohort. Postinstall must also defer manifest hydration and
service finalization: the public manifest still selects the previous package
until publication, while the old updater already owns the exact preverified
candidate and the previous service must remain usable until activation. The old
updater can then activate that candidate and request the managed restart after
dpkg has completed.
"""


def _function(source: str, signature: str) -> str:
    start = source.index(signature)
    opening = source.index("{", start)
    depth = 0
    for index in range(opening, len(source)):
        if source[index] == "{":
            depth += 1
        elif source[index] == "}":
            depth -= 1
            if depth == 0:
                return source[start : index + 1]
    raise AssertionError(f"unterminated function: {signature}")


def _ownership_violations(body: str) -> list[str]:
    required = (
        'program: "systemd-run".to_string()',
        '"--user".to_string()',
        '"--wait".to_string()',
        '"--collect".to_string()',
        '"--unit=capsem-update".to_string()',
        '"--".to_string()',
        "transient_args.extend(args)",
    )
    violations: list[str] = [
        f"missing `{needle}`" for needle in required if needle not in body
    ]
    if '"--pipe"' in body:
        violations.append("uses `--pipe`, tying the updater back to capsem.service")
    if (
        "transient_args.extend(args)" in body
        and "program," in body
        and body.index("program,") > body.index("transient_args.extend(args)")
    ):
        violations.append("places the capsem program after its update arguments")
    return violations


def _systemd_detection_violations(body: str) -> list[str]:
    required = (
        "invocation_id.is_some_and(|value| !value.is_empty())",
        ".and_then(OsStr::to_str)",
        "value.parse::<u32>()",
        "== Some(current_pid)",
    )
    violations: list[str] = [
        f"missing `{needle}`" for needle in required if needle not in body
    ]
    return violations


def _package_handoff_violations(preinstall: str, ownership: str) -> list[str]:
    required_ownership = (
        "capsem_install_runs_inside_service()",
        'grep -Eq \'(^|/)capsem[.]service($|/)\' "$cgroup_file"',
    )
    violations: list[str] = [
        f"missing `{needle}` from package cgroup detection"
        for needle in required_ownership
        if needle not in ownership
    ]
    if "if capsem_install_runs_inside_service /proc/self/cgroup; then" not in preinstall:
        violations.append("preinstall does not detect an old-service-owned dpkg transaction")
        return violations
    branch = preinstall[preinstall.index("if capsem_install_runs_inside_service") :]
    preserve, ordinary = branch.split("else", maxsplit=1)
    ordinary, _ = ordinary.rsplit("\nfi", maxsplit=1)
    if "event=preserve_service_owned_update" not in preserve:
        violations.append("preinstall does not report the preserved self-update handoff")
    for destructive in (
        "systemctl --user stop capsem.service",
        "capsem_retire_native_cohort",
    ):
        if destructive in preserve:
            violations.append(f"service-owned package transaction still runs `{destructive}`")
        if destructive not in ordinary:
            violations.append(f"ordinary package replacement lost `{destructive}`")
    return violations


def _managed_restart_violations(service: str, install: str) -> list[str]:
    required = (
        (service, "selected_binary != state.current_version"),
        (service, "state.update_restart.notify_one()"),
        (install, "Restart=always"),
        (install, '.args(["--user", "enable", "--now", "capsem"])'),
    )
    return [f"missing managed restart rail `{needle}`" for source, needle in required if needle not in source]


def _postinstall_handoff_violations(postinstall: str) -> list[str]:
    guard = "if capsem_install_runs_inside_service /proc/self/cgroup; then"
    if guard not in postinstall:
        return ["postinstall does not detect an old-service-owned dpkg transaction"]
    start = postinstall.index(guard)
    branch_end = postinstall.index("\nfi\n", start)
    deferred = postinstall[start:branch_end]
    ordinary = postinstall[branch_end:]
    violations = []
    if "event=defer_service_owned_manifest_activation" not in deferred:
        violations.append("postinstall does not report deferred candidate activation")
    if "event=defer_service_owned_service_finalization" not in deferred:
        violations.append("postinstall does not report deferred service finalization")
    if "exit 0" not in deferred:
        violations.append("service-owned postinstall still reaches service registration")
    for command in ("update --assets", "update --check"):
        if command in deferred:
            violations.append(f"service-owned postinstall still runs `{command}`")
        if command not in ordinary:
            violations.append(f"ordinary postinstall lost `{command}`")
    for phase in ('CAPSEM_INSTALL_PHASE="register_service"', "event=readiness_poll"):
        if phase not in ordinary:
            violations.append(f"ordinary postinstall lost `{phase}`")
    return violations


def test_systemd_update_owns_the_complete_transaction_outside_capsem_service() -> None:
    source = UPDATE_COMMAND.read_text()
    body = _function(
        source,
        "fn update_command_plan_for(",
    )
    violations = _ownership_violations(body)
    detection = _function(source, "fn direct_systemd_invocation(")
    violations.extend(_systemd_detection_violations(detection))
    if 'std::env::var_os("INVOCATION_ID")' not in source:
        violations.append("does not identify service execution through INVOCATION_ID")
    if 'std::env::var_os("SYSTEMD_EXEC_PID")' not in source:
        violations.append("does not distinguish the directly executed service from descendants")
    if "std::process::id()" not in source:
        violations.append("does not compare SYSTEMD_EXEC_PID with the current service process")
    violations.extend(
        _package_handoff_violations(
            DEB_PREINSTALL.read_text(),
            SERVICE_OWNERSHIP.read_text(),
        )
    )
    violations.extend(
        _managed_restart_violations(
            SERVICE_MAIN.read_text(),
            SERVICE_INSTALL.read_text(),
        )
    )
    assert not violations, SYSTEMD_UPDATE_OWNERSHIP_RATIONALE + "\n" + "\n".join(violations)


def test_service_owned_postinstall_defers_manifest_activation_to_old_updater() -> None:
    violations = _postinstall_handoff_violations(DEB_POSTINSTALL.read_text())
    assert not violations, SYSTEMD_UPDATE_OWNERSHIP_RATIONALE + "\n" + "\n".join(violations)


def test_release_skills_document_the_old_service_package_handoff() -> None:
    preinstall = DEB_PREINSTALL.read_text()
    postinstall = DEB_POSTINSTALL.read_text()
    ownership = SERVICE_OWNERSHIP.read_text()
    cohort = INSTALL_COHORT.read_text()
    repack = REPACK_DEB.read_text()
    installation_skill = " ".join(INSTALLATION_SKILL.read_text().split())
    release_skill = " ".join(
        (RELEASE_SKILL.read_text() + RELEASE_CI_INVARIANTS.read_text()).split()
    )

    for maintainer_script in (preinstall, postinstall):
        assert "build_system/packaging/shared/service-owned-update" in maintainer_script
    assert "capsem_install_runs_inside_service()" in ownership
    assert repack.count('embed_pkg_script service-owned-update "$WORK_DIR/deb/DEBIAN/') == 2
    assert "build_system/packaging/shared/retire-cohort" in preinstall
    assert '"$kill_command" -9 "$pid"' in cohort
    assert 'cp "$SCRIPT_DIR/deb-preinst.sh" "$WORK_DIR/deb/DEBIAN/preinst"' in repack
    assert "embed_native_cohort_retirement" in repack
    assert "preinst plus postinst scripts" in repack
    assert "DEBIAN/preinst script" in repack
    assert "systemctl --user stop capsem.service" in preinstall
    assert "event=preserve_service_owned_update" in preinstall

    for skill in (installation_skill, release_skill):
        assert "deb-preinst.sh" in skill
        assert "DEBIAN/preinst" in skill
        assert "systemctl --user stop capsem.service" in skill
        assert "stale helper cohort before package replacement" in skill
        assert "/proc/self/cgroup" in skill
        assert "postinstall" in skill.lower()
        assert "defers manifest hydration" in skill
        assert "service registration" in skill
        assert "readiness" in skill
    assert "preserves that unit and cohort" in installation_skill
    assert "preserves the old cohort" in release_skill


def test_guard_rejects_the_failure_shapes_that_reached_release_qualification() -> None:
    good = _function(
        UPDATE_COMMAND.read_text(),
        "fn update_command_plan_for(",
    )
    bad_shapes = {
        "direct service child": good.replace(
            'program: "systemd-run".to_string()',
            "program: program.clone()",
        ),
        "pipe back to stopped service": good.replace(
            '"--wait".to_string()',
            '"--wait".to_string(), "--pipe".to_string()',
        ),
        "anonymous unit permits duplicate": good.replace(
            '"--unit=capsem-update".to_string()',
            '"--property=Type=exec".to_string()',
        ),
        "apt-only wrapper": good.replace(
            "transient_args.extend(args)",
            'transient_args.extend(["apt-get".to_string()])',
        ),
    }
    undetected = [name for name, source in bad_shapes.items() if not _ownership_violations(source)]
    assert not undetected, f"ownership guard accepts known failure shapes: {undetected}"

    detection = _function(UPDATE_COMMAND.read_text(), "fn direct_systemd_invocation(")
    inherited_ancestor = detection.replace(
        "== Some(current_pid)",
        ".is_some()",
    )
    assert _systemd_detection_violations(inherited_ancestor), (
        "ownership guard accepts an inherited systemd ancestor identity"
    )

    preinstall = DEB_PREINSTALL.read_text()
    ownership = SERVICE_OWNERSHIP.read_text()
    self_killing_package = preinstall.replace(
        "if capsem_install_runs_inside_service /proc/self/cgroup; then",
        "if false; then",
    )
    assert _package_handoff_violations(self_killing_package, ownership), (
        "ownership guard accepts a package that kills an update launched by 0.6.1"
    )

    postinstall = DEB_POSTINSTALL.read_text()
    stale_public_hydration = postinstall.replace(
        "event=defer_service_owned_manifest_activation",
        "event=defer_service_owned_manifest_activation update --assets",
    )
    assert _postinstall_handoff_violations(stale_public_hydration), (
        "ownership guard accepts candidate hydration before the old updater resumes"
    )

    premature_service_finalization = postinstall.replace(
        "    exit 0\nfi\n\nMANIFEST_SELECTION",
        "    :\nfi\n\nMANIFEST_SELECTION",
    )
    assert premature_service_finalization != postinstall
    assert _postinstall_handoff_violations(premature_service_finalization), (
        "ownership guard accepts service registration before the old updater resumes"
    )
