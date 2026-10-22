"""All official apps share the existing base's graphical runtime."""

from pathlib import Path

import pytest
import yaml

from images.ci.catalog import surfaces

ROOT = Path(__file__).resolve().parents[2]
RATIONALE = (
    "Xpra, Xdummy, keyring, CA setup and graphical launch settings belong in "
    "images/base. Per-app copies or a separate GUI parent drift and repeat "
    "build/download work. App images only select/install their application; "
    "terminal images retain their shell command and terminal surface."
)


def test_existing_base_installs_and_ships_the_graphical_runtime() -> None:
    base = ROOT / "images/base"
    source = (base / "Dockerfile").read_text()
    assert "COPY install-xpra.sh /tmp/install-xpra.sh" in source, RATIONALE
    assert 'RUN sh /tmp/install-xpra.sh "$TARGETARCH"' in source, RATIONALE
    assert "COPY --chmod=0555 gui-entrypoint.sh /usr/local/bin/capsem-gui" in source, RATIONALE
    assert 'CMD ["/bin/bash"]' in source, RATIONALE
    assert 'org.capsem.surface="xpra"' not in source, RATIONALE
    assert not (ROOT / "images/gui").exists(), RATIONALE


@pytest.mark.parametrize("name", [name for name, surface in surfaces().items() if surface == "xpra"])
def test_graphical_apps_only_select_the_shared_launcher(name: str) -> None:
    source = (ROOT / "images" / name / "Dockerfile").read_text()
    assert 'CMD ["/usr/local/bin/capsem-gui"]' in source, RATIONALE
    assert "FROM gui" not in source and "install-xpra" not in source, RATIONALE
    assert "entrypoint.sh" not in source, RATIONALE
    assert "ENV CAPSEM_GUI_APP=" in source, RATIONALE


def _owns_graphical_runtime(path: Path, source: str) -> bool:
    runtime = ("xpra start", "XPRA_VERSION=", "--xvfb=")
    return not any(token in source for token in runtime) or path.parent == ROOT / "images/base"


def test_only_the_existing_base_owns_xpra_configuration() -> None:
    for path in (ROOT / "images").rglob("*"):
        if not path.is_file() or path.suffix != ".sh":
            continue
        assert _owns_graphical_runtime(path, path.read_text()), f"{path}: {RATIONALE}"


@pytest.mark.parametrize("fragment", ["xpra start :100", "XPRA_VERSION=6.5.1", "--xvfb=Xdummy"])
def test_per_app_runtime_copies_are_rejected(fragment: str) -> None:
    assert not _owns_graphical_runtime(ROOT / "images/another-app/launch.sh", fragment), RATIONALE
    assert not _owns_graphical_runtime(ROOT / "images/gui/install.sh", fragment), RATIONALE
    assert _owns_graphical_runtime(ROOT / "images/base/install.sh", fragment), RATIONALE


def test_ci_uses_the_same_base_digest_without_extra_gui_contexts() -> None:
    source = (ROOT / ".github/workflows/images.yaml").read_text()
    workflow = yaml.safe_load(source)
    job = workflow["jobs"]["image-arch"]
    assert job["needs"] == "base", RATIONALE
    assert job["env"]["BASE_DIGEST"] == "${{ needs.base.outputs.digest }}", RATIONALE
    assert '--build-arg "BASE=$REGISTRY/base@$BASE_DIGEST"' in source, RATIONALE
    assert "--with \"$BASE_DIGEST\"" in source, RATIONALE
    assert "images/gui" not in source and "--build-context gui=" not in source, RATIONALE
