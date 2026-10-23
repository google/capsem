"""Only the supported AGY recipe requests the fixed scratch workspace policy."""

from pathlib import Path


def test_scratch_policy_is_recipe_owned_and_keeps_the_existing_home_volume():
    root = Path(__file__).resolve().parents[2] / "images"
    declared = []
    for recipe in sorted(root.glob("*/Dockerfile")):
        text = recipe.read_text()
        if "org.capsem.workspace.policy" in text:
            declared.append(recipe.parent.name)
            assert 'org.capsem.workspace.policy="antigravity-scratch-v1"' in text
            assert "VOLUME /home/capsem" in text
    assert declared == ["antigravity-desktop"]
