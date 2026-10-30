"""The functional suites drive every SDK, so the functional module prepares them.

Braavos installs the Python wheel/sdist and TypeScript tarball in clean external
consumers, then drives those packages and the source-built Rust SDK through a
real gateway. The sandboxed suites need this source's package archives and
prewarmed runtime dependencies; an editable checkout or a previous run's
archives cannot prove the distributed packages work.
"""

from __future__ import annotations

from capsem_builder.gate import config as gate_config
from capsem_builder.gate import host
from capsem_builder.gate.actions import Run
from capsem_builder.gate.content import RuntimeContent
from capsem_builder.gate.module_glowup import glowup
from capsem_builder.gate.plan import Plan
from capsem_builder.gate.qualification import BinaryQualification, RuntimeQualification
from helpers.gate import PROJECT_ROOT, gate_plan

PREPARED = (
    "sdk.python.sync", "functional.sdk.rust.example", "fast.sdk.python.build",
    "fast.sdk.typescript.build", "fast.sdk.typescript.package-prewarm",
    "functional.sdk.python.package-prewarm",
    "integrations.inspect-ai.sync", "fast.integrations.inspect-ai.build",
)


def _ancestors(plan, label: str) -> set[str]:
    seen: set[str] = set()
    pending = [label]
    while pending:
        for before in plan.after_of(pending.pop()):
            if before not in seen:
                seen.add(before)
                pending.append(before)
    return seen


def test_inspect_acceptance_has_its_own_two_worker_fleet_without_changing_broad() -> None:
    for qualification in (
        None,
        BinaryQualification(input_dir="/staged", package="/candidate.deb", bin_dir="/bin"),
        RuntimeQualification(input_dir="/staged", package="/candidate.deb", bin_dir="/bin"),
    ):
        plan = gate_plan("test-functional", qualification=qualification)
        inspect = plan.step_named("functional.pytest.inspect")
        broad = plan.step_named("functional.pytest.broad")
        assert broad.label in _ancestors(plan, inspect.label)
        assert inspect.concurrency == 2
        assert broad.concurrency == 4
        assert "-n 2" in inspect.actions[0].render()
        assert "-n 4" in broad.actions[0].render()
        config = gate_config.load(PROJECT_ROOT)
        for node in config.suites.pytest.inspect_live:
            assert f"--deselect={node}" in broad.actions[0].render()
            assert node in inspect.actions[0].render()
        assert config.exclusive("vm_fleet") in inspect.contends


def test_logger_restart_target_is_prepared_before_concurrent_vm_acceptance() -> None:
    """A cold compiler must not spend the logger's bounded correctness-test budget."""
    for qualification in (
        None,
        BinaryQualification(input_dir="/staged", package="/candidate.deb", bin_dir="/bin"),
        RuntimeQualification(input_dir="/staged", package="/candidate.deb", bin_dir="/bin"),
    ):
        plan = gate_plan("test-functional", qualification=qualification)
        label = "functional.logger-rehydration.compile"
        assert label in _ancestors(plan, "functional.pytest.broad")
        action = plan.step_named(label).actions[0]
        config = gate_config.load(PROJECT_ROOT)
        assert action.render() == " ".join(
            (*config.functional.logger_rehydration_cargo, "--no-run")
        ) + f" [timeout {config.functional.logger_rehydration_build_timeout_seconds}s]"
        assert "--frozen" in action.render()
        assert "db_query_exact_after_flush_and_restart" in action.render()


def test_python_sdk_tests_cannot_start_before_the_archives_they_install_are_built() -> None:
    plan = gate_plan("test-fast")
    assert "fast.sdk.python.build" in _ancestors(plan, "fast.sdk.python.tests"), (
        "clean installed-package acceptance must consume this source's wheel and sdist, "
        "not whichever archive a previous run happened to leave"
    )
    assert {"fast.sdk.python.build", "fast.integrations.inspect-ai.build"} <= _ancestors(
        plan, "fast.integrations.inspect-ai.tests"
    ), (
        "clean installed inspect-capsem acceptance must consume this source's SDK and "
        "inspect-capsem wheel and sdist archives"
    )


def test_python_projects_cannot_empty_each_others_or_the_shared_pytest_scratch() -> None:
    import shlex

    config = gate_config.load(PROJECT_ROOT)
    plan = gate_plan("test-fast")
    scratch_paths = []
    for label, settings in (
        ("fast.sdk.python.tests", config.sdk_python),
        ("fast.integrations.inspect-ai.tests", config.integrations_inspect_ai),
    ):
        commands = [action for action in plan.step_named(label).actions if isinstance(action, Run)]
        assert len(commands) == 1
        argv = shlex.split(commands[0].render())
        assert "--basetemp" in argv, (
            "separate Python projects must override inherited basetemp rather than "
            "deleting concurrent Citadel tmp_path directories"
        )
        scratch = argv[argv.index("--basetemp") + 1]
        assert scratch == str(config.path(settings.pytest_scratch))
        scratch_paths.append(scratch)
    assert scratch_paths[0] != scratch_paths[1]


def test_typescript_package_acceptance_has_network_prewarm_outside_the_sandbox() -> None:
    from capsem_builder.gate.execution import Needs

    plan = gate_plan("test-fast")
    label = "fast.sdk.typescript.package-prewarm"
    assert label in plan.labels, "offline clean-consumer installs need declared dependency prewarming"
    warmed = plan.step_named(label)
    assert Needs.NETWORK in warmed.needs
    assert "[outside kernel sandbox]" in warmed.actions[0].render()
    assert {label, "fast.sdk.typescript.build"} <= _ancestors(plan, "fast.sdk.typescript.tests")


def test_every_functional_suite_starts_after_the_sdks_are_prepared() -> None:
    plan = gate_plan("test-functional")
    suites = [label for label in plan.labels if ".pytest." in label]
    assert suites, "the functional plan has no pytest suites to check"
    for suite in suites:
        missing = [step for step in PREPARED if step not in _ancestors(plan, suite)]
        assert not missing, f"{suite} can start before {missing}"


def test_functional_installed_sdks_use_current_archives_and_declared_network_prewarm() -> None:
    from capsem_builder.gate.execution import Needs

    plan = gate_plan("test-functional")
    python = "fast.sdk.python.build"
    inspect_pkg = "fast.integrations.inspect-ai.build"
    npm = "fast.sdk.typescript.build"
    warmed = "fast.sdk.typescript.package-prewarm"
    assert {python, inspect_pkg, npm, warmed} <= set(plan.labels), "Braavos needs actual installed SDK artifacts"
    assert "sdk.python.sync" in _ancestors(plan, python)
    assert "integrations.inspect-ai.sync" in _ancestors(plan, inspect_pkg)
    assert npm in _ancestors(plan, warmed)
    assert Needs.NETWORK in plan.step_named(warmed).needs
    assert "[outside kernel sandbox]" in plan.step_named(warmed).actions[0].render()
    assert "--no-isolation" in plan.step_named(python).actions[0].render()
    assert "--no-isolation" in plan.step_named(inspect_pkg).actions[0].render()


def test_candidate_rehydrates_python_build_backends_after_cache_enforcement() -> None:
    from capsem_builder.gate.execution import Needs

    plan = gate_plan("candidate")
    label = "functional.sdk.python.package-prewarm"
    warmed = plan.step_named(label)
    assert {"fast.sdk.python.build", "prepare.cache-enforcement"} <= _ancestors(plan, label)
    assert Needs.NETWORK in warmed.needs
    exported, installed = (action.render() for action in warmed.actions)
    assert "--only-group package-build" in exported
    assert "--reinstall" in installed and "[outside kernel sandbox]" in installed


def test_composed_sdk_packages_have_one_producer_and_no_dependency_cycle() -> None:
    plan = gate_plan("candidate")
    labels = plan.labels
    for label in ("fast.sdk.python.build", "fast.integrations.inspect-ai.build",
                  "fast.sdk.typescript.build", "fast.sdk.typescript.package-prewarm"):
        assert labels.count(label) == 1


def test_mcp_package_acceptance_prewarms_after_both_current_packages_build() -> None:
    from capsem_builder.gate.execution import Needs

    plan = gate_plan("test-fast")
    label = "fast.mcp.typescript.package-prewarm"
    assert label in plan.labels, "offline installed MCP acceptance needs its declared dependency prewarm"
    warmed = plan.step_named(label)
    assert Needs.NETWORK in warmed.needs
    assert "[outside kernel sandbox]" in warmed.actions[0].render()
    config = gate_config.load(PROJECT_ROOT)
    sdk_archives = config.path(config.sdk_typescript.build_output)
    assert f"CAPSEM_SDK_PACKAGE_ARCHIVES={sdk_archives}" in warmed.actions[0].render(), (
        "MCP acceptance must consume the SDK archive that publication validates, without repacking it"
    )
    assert {"fast.sdk.typescript.build", "fast.mcp.typescript.build"} <= _ancestors(plan, label)
    assert label in _ancestors(plan, "fast.mcp.typescript.tests")


def test_the_packages_the_suites_drive_are_among_the_installed_workspaces() -> None:
    config = gate_config.load(PROJECT_ROOT)
    for project in (config.sdk_typescript.project, config.mcp_typescript.project):
        assert project in config.functional.node_workspaces, project


def test_sdk_and_mcp_acceptance_select_the_published_sdk_archive_directory() -> None:
    config = gate_config.load(PROJECT_ROOT)
    variable = config.environment.model_dump().get("sdk_package_archives")
    assert variable, "one config-owned archive selection must reach both SDK consumers"
    plan = gate_plan("test-fast")
    for label in ("fast.sdk.typescript.tests", "fast.mcp.typescript.package-prewarm"):
        command = plan.step_named(label).actions[0].render()
        assert f"{variable}={config.path(config.sdk_typescript.build_output)}" in command, (
            "acceptance must consume the publication SDK bytes, not independently repack them"
        )


def test_sdk_preparation_stays_offline_inside_the_sandbox() -> None:
    plan = gate_plan("test-functional")
    for label in ("sdk.python.sync", "integrations.inspect-ai.sync"):
        sync = plan.step_named(label).actions[0].render()
        assert "--no-build-isolation" in sync, (
            "an isolated build fetches its backend from the network"
        )
    example = plan.step_named("functional.sdk.rust.example").actions[0].render()
    assert "--frozen" in example, "cargo must not reach the registry from inside the sandbox"


def test_native_supervisor_restart_is_owned_by_glowup_on_both_platforms() -> None:
    """Native supervisor setup precedes inherited privilege confinement;
    Linux then confines its test worker to the manager's closed network."""
    config = gate_config.load(PROJECT_ROOT)
    managed = config.suites.pytest.managed_restart
    assert managed in config.suites.pytest.broad_ignores
    plan = gate_plan("test-glowup")
    label = "glowup.pytest.managed-restart"
    rendered = plan.step_named(label).actions[0].render()
    assert managed in rendered and "[outside kernel sandbox]" in rendered
    assert f"{config.environment.assets_dir}={RuntimeContent.built(config).assets} " in rendered
    assert "sdk.python.sync" in _ancestors(plan, label)
    assert "sdk.typescript.bundle" in _ancestors(plan, label)
    if host.on_macos():
        assert "prepare.sign" in _ancestors(plan, label)
    else:
        assert config.suites.pytest.managed_restart_driver in rendered


def test_native_restart_rejects_a_broken_host_before_cross_compiling_packages() -> None:
    plan = gate_plan("test-glowup")
    for arch in gate_config.load(PROJECT_ROOT).architectures:
        assert "glowup.pytest.managed-restart" in _ancestors(plan, f"package.{arch}.build"), (
            "Managed restart consumes the prepared host cohort, not cross-compiled packages; "
            "a startup failure must surface before spending hours building both architectures."
        )


def test_pulled_package_lanes_keep_native_restart_and_sdk_prerequisites() -> None:
    config = gate_config.load(PROJECT_ROOT)
    for owner in (BinaryQualification, RuntimeQualification):
        qualification = owner(input_dir="/staged", package="/candidate.deb", bin_dir="/candidate-bin")
        plan = gate_plan("test-glowup", qualification=qualification)
        label = "glowup.pytest.managed-restart"
        assert {"sdk.python.sync", "sdk.typescript.bundle"} <= _ancestors(plan, label)
        assert label in _ancestors(plan, "glowup.package")
        rendered = plan.step_named(label).actions[0].render()
        assert config.suites.pytest.managed_restart in rendered
        assert f"{config.environment.assets_dir}={RuntimeContent.standalone(config).assets} " in rendered


def test_pulled_restart_uses_the_handed_runtime_assets() -> None:
    config = gate_config.load(PROJECT_ROOT)
    content = RuntimeContent.staged(config, PROJECT_ROOT / "cache/target/selected-cohort")
    for owner in (BinaryQualification, RuntimeQualification):
        qualification = owner(input_dir="/staged", package="/candidate.deb", bin_dir="/candidate-bin")
        plan = Plan("selected-cohort")
        glowup(plan, config, qualification=qualification, staged=content)
        rendered = plan.step_named("glowup.pytest.managed-restart").actions[0].render()
        assert f"{config.environment.assets_dir}={content.assets} " in rendered


def test_candidate_restart_reuses_the_prepared_sdk_dependencies() -> None:
    plan = gate_plan("candidate")
    assert "fast.toolchain.node" in _ancestors(plan, "sdk.typescript.bundle")
    assert "toolchain.node" not in plan.labels
