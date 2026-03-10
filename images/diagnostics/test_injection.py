"""Data-driven injection verification.

Reads /tmp/capsem-injection-manifest.json (written by the host during boot config)
and verifies every env var and boot file arrived correctly inside the guest.

The manifest is always written by send_boot_config(), so these tests run during
any `capsem-doctor -k injection` invocation. They skip gracefully if the manifest
is missing (e.g., running an older capsem binary).
"""
import json
import os
import stat

import pytest

from conftest import run

MANIFEST_PATH = "/tmp/capsem-injection-manifest.json"


def _load_manifest():
    if not os.path.isfile(MANIFEST_PATH):
        pytest.skip("no injection manifest (not running under injection harness)")
    with open(MANIFEST_PATH) as f:
        return json.load(f)


def _shell_overridden_vars():
    """Find env var names that guest scripts set after boot config injection.
    Scans capsem-init, .bashrc, and capsem-doctor for `export VAR=` lines.
    These vars are legitimately modified, so exact-value comparison is wrong."""
    scripts = [
        "/root/.bashrc",
        "/usr/local/bin/capsem-doctor",   # rootfs path
        "/capsem-doctor",                 # initrd path
    ]
    names = set()
    for path in scripts:
        if not os.path.isfile(path):
            continue
        with open(path) as f:
            for line in f:
                line = line.strip()
                if line.startswith("#") or not line:
                    continue
                if line.startswith("export "):
                    token = line[len("export "):].split("=")[0].strip()
                    if token.isidentifier():
                        names.add(token)
    return names


# -- Env vars --


class TestEnvVars:
    def test_all_env_vars_present(self):
        """Every env var the host sent must be set in the guest environment.
        Vars that .bashrc modifies are checked for presence only (not exact
        value) since the shell legitimately overrides them after injection."""
        m = _load_manifest()
        overridden = _shell_overridden_vars()
        missing = []
        for key, expected in m["env"].items():
            actual = os.environ.get(key)
            if key in overridden:
                # .bashrc modifies this var -- just check it exists.
                if actual is None:
                    missing.append(f"{key}: not set (bashrc-modified, presence check)")
            elif actual != expected:
                missing.append(f"{key}: expected={expected!r}, got={actual!r}")
        assert not missing, "env var mismatches:\n" + "\n".join(missing)

    def test_no_empty_env_vars(self):
        """Env vars in the manifest should never have empty values."""
        m = _load_manifest()
        empty = [k for k, v in m["env"].items() if v == ""]
        assert not empty, f"env vars with empty values: {empty}"


# -- Boot files --


class TestBootFiles:
    def test_all_files_exist(self):
        """Every file the host sent must exist in the guest filesystem."""
        m = _load_manifest()
        missing = []
        for f in m["files"]:
            if not os.path.isfile(f["path"]):
                missing.append(f["path"])
        assert not missing, f"missing boot files: {missing}"

    def test_file_permissions(self):
        """Boot files must have exactly the permissions specified in the manifest."""
        m = _load_manifest()
        bad = []
        for f in m["files"]:
            if not os.path.isfile(f["path"]):
                continue
            actual = stat.S_IMODE(os.stat(f["path"]).st_mode)
            expected = f["mode"]
            if actual != expected:
                bad.append(f"{f['path']}: {oct(actual)} != {oct(expected)}")
        assert not bad, "permission mismatches:\n" + "\n".join(bad)

    def test_files_non_empty(self):
        """Boot files should contain data (not be zero-length)."""
        m = _load_manifest()
        empty = []
        for f in m["files"]:
            path = f["path"]
            if os.path.isfile(path) and os.path.getsize(path) == 0:
                empty.append(path)
        assert not empty, f"empty boot files: {empty}"


# -- .git-credentials --


class TestGitCredentials:
    def test_git_credentials_format(self):
        """If .git-credentials was injected, every line must be a valid credential URL."""
        m = _load_manifest()
        cred_files = [f for f in m["files"] if f["path"] == "/root/.git-credentials"]
        if not cred_files:
            pytest.skip("no .git-credentials in manifest")
        content = open("/root/.git-credentials").read()
        for line in content.strip().splitlines():
            assert line.startswith("https://"), f"credential line must start with https://: {line}"
            assert "@" in line, f"credential line must contain @: {line}"
            # Expected format: https://oauth2:TOKEN@HOST
            parts = line.split("@", 1)
            assert parts[0].startswith("https://oauth2:"), (
                f"credential line must use oauth2 login: {line}"
            )
            host = parts[1]
            assert host, f"empty host in credential line: {line}"

    def test_git_credentials_permissions(self):
        """If .git-credentials was injected, it must be owner-only (0600)."""
        m = _load_manifest()
        cred_files = [f for f in m["files"] if f["path"] == "/root/.git-credentials"]
        if not cred_files:
            pytest.skip("no .git-credentials in manifest")
        actual = stat.S_IMODE(os.stat("/root/.git-credentials").st_mode)
        assert actual == 0o600, f".git-credentials permissions: {oct(actual)} != 0o600"

    def test_gitconfig_exists(self):
        """If .git-credentials exists, .gitconfig must also exist with credential.helper."""
        m = _load_manifest()
        cred_files = [f for f in m["files"] if f["path"] == "/root/.git-credentials"]
        if not cred_files:
            pytest.skip("no .git-credentials in manifest")
        assert os.path.isfile("/root/.gitconfig"), ".gitconfig must exist alongside .git-credentials"
        content = open("/root/.gitconfig").read()
        assert "helper = store" in content, ".gitconfig must set credential.helper = store"

    def test_git_credential_fill(self):
        """git credential fill must return the token for each configured host."""
        m = _load_manifest()
        cred_files = [f for f in m["files"] if f["path"] == "/root/.git-credentials"]
        if not cred_files:
            pytest.skip("no .git-credentials in manifest")
        content = open("/root/.git-credentials").read()
        for line in content.strip().splitlines():
            # Parse https://oauth2:TOKEN@HOST
            parts = line.split("@", 1)
            host = parts[1]
            result = run(
                f'echo "protocol=https\nhost={host}\n" | git credential fill',
                timeout=5,
            )
            assert "password=" in result.stdout, (
                f"git credential fill failed for {host}: {result.stdout}"
            )


# -- GitHub CLI --


class TestGitHubCli:
    def test_gh_token_set(self):
        """GH_TOKEN env var must be set when GitHub is enabled with a token."""
        m = _load_manifest()
        env = m["env"]
        if "GH_TOKEN" not in env:
            pytest.skip("GH_TOKEN not in manifest (GitHub not configured)")
        actual = os.environ.get("GH_TOKEN")
        assert actual, "GH_TOKEN env var is not set in the guest"

    def test_gh_auth_status(self):
        """gh auth status must detect the GH_TOKEN env var.

        Injection tests use fake tokens, so authentication failure is expected.
        We only verify that gh detected GH_TOKEN and attempted to use it.
        """
        if not os.environ.get("GH_TOKEN"):
            pytest.skip("GH_TOKEN not set")
        result = run("gh auth status", timeout=10)
        output = result.stdout + result.stderr
        # gh auth status should mention github.com and GH_TOKEN regardless of
        # whether the token is valid (injection tests use fake tokens).
        assert "github.com" in output, (
            f"gh did not detect github.com: {output}"
        )
        assert "GH_TOKEN" in output, (
            f"gh did not detect GH_TOKEN env var: {output}"
        )
