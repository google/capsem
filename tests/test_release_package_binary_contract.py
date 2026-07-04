"""Release package and executable inventory contract gates."""

from __future__ import annotations

import fcntl
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path

import blake3

from test_release_site_html_contract import RELEASE_SITE_DIST, build_release_site_from_fixture


PROJECT_ROOT = Path(__file__).resolve().parents[1]
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)
FIXTURE_FILE_ROOT = PROJECT_ROOT / "tests" / "capsem-release" / "fixtures" / "release-channel-files"
EXPECTED_BINARY_COHORT = {
    "capsem",
    "capsem-admin",
    "capsem-app",
    "capsem-gateway",
    "capsem-mcp",
    "capsem-mcp-aggregator",
    "capsem-mcp-builtin",
    "capsem-process",
    "capsem-service",
    "capsem-tray",
    "capsem-tui",
}
EXPECTED_MACOS_BINARY_PATHS = {
    "capsem-app": "/Applications/Capsem.app/Contents/MacOS/capsem-app",
    **{
        name: f"/usr/local/share/capsem/bin/{name}"
        for name in EXPECTED_BINARY_COHORT
        if name != "capsem-app"
    },
}
EXPECTED_LINUX_BINARY_PATHS = {
    name: f"/usr/bin/{name}" for name in EXPECTED_BINARY_COHORT
}


def build_release_site_from_graph(graph_path: Path) -> None:
    lock_path = Path(os.environ.get("TMPDIR", "/tmp")) / "capsem-release-site-build.lock"
    with lock_path.open("w", encoding="utf-8") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        env = {
            **os.environ,
            "ASTRO_TELEMETRY_DISABLED": "1",
            "CAPSEM_RELEASE_CHANNEL_DIST": str(graph_path),
        }
        result = subprocess.run(
            ["pnpm", "--dir", "release-site", "run", "build"],
            cwd=PROJECT_ROOT,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    assert result.returncode == 0, result.stdout + result.stderr
    build_release_site_from_fixture.cache_clear()


def test_package_owns_binaries() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        packages = manifest["packages"]

        assert packages, channel
        for package in packages:
            assert "name" in package, package
            assert "url" in package, package
            assert "digest" in package, package
            assert package["binaries"], package["name"]
            for binary in package["binaries"]:
                assert "package" not in binary, binary
                assert binary["name"], binary
                assert binary["version"], binary
                assert binary["installed_path"].startswith("/"), binary
                assert len(binary["digest"]["sha256"]) == 64, binary
                assert len(binary["digest"]["blake3"]) == 64, binary
                assert binary["sbom_component_ref"].startswith("SPDXRef-"), binary


def test_sbom_not_repeated_per_binary() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for package in manifest["packages"]:
            package_sboms = [
                item
                for item in package.get("evidence", [])
                if "sbom" in item["kind"].lower()
            ]
            assert package_sboms, package["name"]
            for binary in package["binaries"]:
                assert "evidence" not in binary, binary
                assert "package_evidence" not in binary, binary
                assert "sbom" not in binary, binary
                assert "sbom_component_ref" in binary, binary


def test_package_sbom_not_repeated_per_binary() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for package in manifest["packages"]:
            package_page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "packages"
                / package["id"]
                / "index.html"
            ).read_text(encoding="utf-8")
            evidence_urls = [item["url"] for item in package["evidence"]]
            binary_refs = [binary["sbom_component_ref"] for binary in package["binaries"]]

            for url in evidence_urls:
                assert package_page.count(url) == 1, f"{channel}:{package['name']}:{url}"
            for ref in binary_refs:
                assert ref in package_page, f"{channel}:{package['name']}:{ref}"

            binary_section = package_page.split("Contained Binaries", maxsplit=1)[1].split(
                "Package Evidence",
                maxsplit=1,
            )[0]
            for url in evidence_urls:
                assert url not in binary_section, f"{channel}:{package['name']}:{url}"
            for binary in package["binaries"]:
                assert "evidence" not in binary
                assert "sbom" not in binary


def test_package_detail_sbom_once_binary_refs() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for package in manifest["packages"]:
            package_page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "packages"
                / package["id"]
                / "index.html"
            ).read_text(encoding="utf-8")
            binaries_section = package_page.split("Contained Binaries", maxsplit=1)[1].split(
                "Package Evidence",
                maxsplit=1,
            )[0]
            evidence_section = package_page.split("Package Evidence", maxsplit=1)[1]
            sboms = [
                item
                for item in package.get("evidence", [])
                if item.get("kind") == "sbom"
            ]

            assert len(sboms) == 1, f"{channel}:{package['name']}"
            sbom_url = sboms[0]["url"]
            assert package_page.count(sbom_url) == 1, f"{channel}:{package['name']}"
            assert sbom_url not in binaries_section, f"{channel}:{package['name']}"
            assert sbom_url in evidence_section, f"{channel}:{package['name']}"

            for binary in package["binaries"]:
                ref = binary["sbom_component_ref"]
                rendered_ref = f"<code>{ref}</code>"
                assert binaries_section.count(rendered_ref) == 1, (
                    f"{channel}:{package['name']}:{binary['name']}"
                )
                assert rendered_ref not in evidence_section, (
                    f"{channel}:{package['name']}:{binary['name']}"
                )


def test_binary_sbom_component_refs_resolve() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for package in manifest["packages"]:
            sboms = [item for item in package["evidence"] if item["kind"] == "sbom"]
            assert len(sboms) == 1, f"{channel}:{package['name']}"
            sbom = sboms[0]
            sbom_path = FIXTURE_FILE_ROOT / sbom["url"].lstrip("/")
            assert sbom_path.exists(), f"{channel}:{package['name']}:{sbom['url']}"

            sbom_bytes = sbom_path.read_bytes()
            assert len(sbom_bytes) == sbom["bytes"], f"{channel}:{package['name']}"
            assert hashlib.sha256(sbom_bytes).hexdigest() == sbom["digest"]["sha256"]
            assert blake3.blake3(sbom_bytes).hexdigest() == sbom["digest"]["blake3"]

            document = json.loads(sbom_bytes)
            assert document["spdxVersion"] == "SPDX-2.3"
            files_by_id = {
                file["SPDXID"]: file
                for file in document.get("files", [])
                if isinstance(file, dict) and "SPDXID" in file
            }
            for binary in package["binaries"]:
                component = files_by_id.get(binary["sbom_component_ref"])
                assert component is not None, (
                    f"{channel}:{package['name']}:{binary['sbom_component_ref']}"
                )
                sha256_checksums = [
                    checksum["checksumValue"]
                    for checksum in component.get("checksums", [])
                    if checksum.get("algorithm") == "SHA256"
                ]
                assert sha256_checksums == [binary["digest"]["sha256"]], (
                    f"{channel}:{package['name']}:{binary['name']}"
                )


def test_packages_group_by_os_architecture() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    stable = (
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    packages_section = stable.split("Capsem Packages", maxsplit=1)[1].split(
        "Profile References",
        maxsplit=1,
    )[0]
    stable_packages = graph["manifests"]["stable"]["1.0.2"]["packages"]
    target_labels = {
        ("macos", "arm64"): "macOS arm64",
        ("linux", "x86_64"): "Linux x86_64",
        ("linux", "arm64"): "Linux arm64",
    }

    assert {
        (package["platform"], package["architecture"]) for package in stable_packages
    } == set(target_labels)
    for label in target_labels.values():
        assert f"Package target {label}" in packages_section
    for package in stable_packages:
        target = (package["platform"], package["architecture"])
        arch_position = packages_section.index(f"Package target {target_labels[target]}")
        package_position = packages_section.index(package["name"])
        assert arch_position < package_position

    assert "Architecture arm64 / macos" not in packages_section
    assert "Architecture arm64 / linux" not in packages_section


def test_manifest_package_targets_by_architecture() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        targets = {
            (package["platform"], package["architecture"])
            for package in manifest["packages"]
        }

        assert targets, channel
        for package in manifest["packages"]:
            assert package["platform"] in {"macos", "linux"}, package
            assert package["architecture"] in {"arm64", "x86_64"}, package
            for binary in package["binaries"]:
                assert binary["platform"] == package["platform"], binary
                assert binary["architecture"] == package["architecture"], binary


def test_package_architecture_sections_are_explicit() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    stable = (
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    packages_section = stable.split("Capsem Packages", maxsplit=1)[1].split(
        "Profile References",
        maxsplit=1,
    )[0]
    stable_packages = graph["manifests"]["stable"]["1.0.2"]["packages"]

    for package in stable_packages:
        platform = "macOS" if package["platform"] == "macos" else package["platform"].title()
        heading = f"Package target {platform} {package['architecture']}"
        assert heading in packages_section
        assert packages_section.index(heading) < packages_section.index(package["name"])


def test_package_architecture_not_filename_derived(tmp_path: Path) -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    stable_packages = graph["manifests"]["stable"]["1.0.2"]["packages"]
    arm64_deb = next(
        package
        for package in stable_packages
        if package["platform"] == "linux" and package["architecture"] == "arm64"
    )
    lying_name = "Capsem_1.4.0_amd64-looking-name.deb"
    arm64_deb["name"] = lying_name
    arm64_deb["url"] = arm64_deb["url"].replace("arm64.deb", "amd64-looking-name.deb")

    graph_path = tmp_path / "release-graph-filename-lies.json"
    graph_path.write_text(json.dumps(graph, indent=2), encoding="utf-8")

    build_release_site_from_graph(graph_path)

    stable = (
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    packages_section = stable.split("Capsem Packages", maxsplit=1)[1].split(
        "Profile References",
        maxsplit=1,
    )[0]
    linux_arm64 = packages_section.split("Package target Linux arm64", maxsplit=1)[1]
    linux_x86_64 = packages_section.split("Package target Linux x86_64", maxsplit=1)[1]

    assert lying_name in linux_arm64
    assert lying_name not in linux_x86_64
    assert "Package target Linux amd64" not in packages_section


def test_package_target_rows_include_own_sbom() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    stable = (
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    packages_section = stable.split("Capsem Packages", maxsplit=1)[1].split(
        "Profile References",
        maxsplit=1,
    )[0]
    stable_packages = graph["manifests"]["stable"]["1.0.2"]["packages"]

    for package in stable_packages:
        platform = "macOS" if package["platform"] == "macos" else package["platform"].title()
        heading = f"Package target {platform} {package['architecture']}"
        target_section = packages_section.split(heading, maxsplit=1)[1]
        next_target = target_section.find("Package target ")
        if next_target >= 0:
            target_section = target_section[:next_target]

        sboms = [item for item in package["evidence"] if item["kind"] == "sbom"]
        assert len(sboms) == 1, package["name"]
        sbom = sboms[0]
        sbom_name = sbom["url"].split("/")[-1]

        assert package["name"] in target_section
        assert sbom_name in target_section
        assert f"{sbom['bytes']:,}" in target_section
        assert sbom["digest"]["sha256"][:8] + "..." in target_section
        assert sbom["digest"]["blake3"][:8] + "..." in target_section


def test_package_target_sbom() -> None:
    test_package_target_rows_include_own_sbom()


def test_every_package_has_sbom() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        sbom_urls = []

        for package in manifest["packages"]:
            sboms = [
                item
                for item in package.get("evidence", [])
                if item.get("kind") == "sbom"
            ]
            assert len(sboms) == 1, package["name"]
            sbom = sboms[0]
            assert package["id"] in sbom["url"], package["name"]
            assert len(sbom["digest"]["sha256"]) == 64, package["name"]
            assert len(sbom["digest"]["blake3"]) == 64, package["name"]
            sbom_urls.append(sbom["url"])

        assert len(sbom_urls) == len(set(sbom_urls)), f"{channel} repeats package SBOM URLs"


def test_package_sbom() -> None:
    test_every_package_has_sbom()


def test_every_package_has_detail_page() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    target_labels = {
        ("macos", "arm64"): "macOS arm64",
        ("linux", "x86_64"): "Linux x86_64",
        ("linux", "arm64"): "Linux arm64",
    }

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for package in manifest["packages"]:
            package_page_path = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "packages"
                / package["id"]
                / "index.html"
            )
            assert package_page_path.exists(), f"{channel}:{package['id']}"

            package_page = package_page_path.read_text(encoding="utf-8")
            package_text = re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", package_page))
            target = (package["platform"], package["architecture"])
            assert f"Package target {target_labels[target]}" in package_text
            assert package["name"] in package_page
            assert package["url"] in package_page
            assert package["digest"]["sha256"][:8] + "..." in package_page
            assert package["digest"]["blake3"][:8] + "..." in package_page

            for evidence in package["evidence"]:
                assert evidence["url"] in package_page
                assert evidence["digest"]["sha256"][:8] + "..." in package_page
                assert evidence["digest"]["blake3"][:8] + "..." in package_page

            for binary in package["binaries"]:
                assert binary["name"] in package_page
                assert binary["installed_path"] in package_page
                assert binary["digest"]["sha256"][:8] + "..." in package_page
                assert binary["digest"]["blake3"][:8] + "..." in package_page
                assert binary["sbom_component_ref"] in package_page

            assert "No binary inventory is published for this package." not in package_page
            assert "No package evidence is published for this package." not in package_page


def test_package_detail_lists_owned_binaries_only() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    packages = graph["manifests"]["stable"]["1.0.2"]["packages"]
    selected = packages[0]
    sibling = packages[1]
    package_page = (
        RELEASE_SITE_DIST
        / "channels"
        / "stable"
        / "packages"
        / selected["id"]
        / "index.html"
    ).read_text(encoding="utf-8")

    assert f"<h1 class=\"mt-3 text-4xl font-semibold tracking-normal text-black\">{selected['name']}</h1>" in package_page
    assert "Capsem Package" not in package_page
    assert selected["name"] in package_page
    assert selected["url"] in package_page
    assert sibling["name"] not in package_page
    assert sibling["url"] not in package_page
    for binary in selected["binaries"]:
        assert binary["name"] in package_page
        assert binary["installed_path"] in package_page
        assert binary["sbom_component_ref"] in package_page
    for binary in sibling["binaries"]:
        assert binary["installed_path"] not in package_page
    for evidence in selected["evidence"]:
        assert evidence["url"] in package_page
    for evidence in sibling["evidence"]:
        assert evidence["url"] not in package_page


def test_package_owns_only_its_binaries() -> None:
    test_package_detail_lists_owned_binaries_only()


def test_package_detail_is_binary_owner_view() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    stable = (
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    packages = graph["manifests"]["stable"]["1.0.2"]["packages"]

    assert "Capsem Packages" in stable
    assert "Capsem Binaries" not in stable

    for package in packages:
        assert package["name"] in stable
        assert f"/channels/stable/packages/{package['id']}/" in stable
        for binary in package["binaries"]:
            assert binary["installed_path"] not in stable
            assert binary["sbom_component_ref"] not in stable

        package_page = (
            RELEASE_SITE_DIST
            / "channels"
            / "stable"
            / "packages"
            / package["id"]
            / "index.html"
        ).read_text(encoding="utf-8")
        assert "Contained Binaries" in package_page
        assert "Package Evidence" in package_page
        for binary in package["binaries"]:
            assert binary["name"] in package_page
            assert binary["installed_path"] in package_page
            assert binary["sbom_component_ref"] in package_page


def test_binary_descriptions_from_metadata() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    forbidden_descriptions = {
        "",
        "Capsem binary package",
        "Capsem executable fixture",
        "unknown",
        "not published",
    }

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for package in manifest["packages"]:
            package_page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "packages"
                / package["id"]
                / "index.html"
            ).read_text(encoding="utf-8")

            assert "Capsem binary package" not in package_page
            assert "Capsem executable fixture" not in package_page

            for binary in package["binaries"]:
                assert binary["description"] not in forbidden_descriptions, binary
                assert binary["description"] in package_page


def test_binaries_inherit_package_target_not_all() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for package in graph["manifests"]["stable"]["1.0.2"]["packages"]:
        for binary in package["binaries"]:
            assert binary["architecture"] == package["architecture"], binary
            assert binary["platform"] == package["platform"], binary

        package_page = (
            RELEASE_SITE_DIST
            / "channels"
            / "stable"
            / "packages"
            / package["id"]
            / "index.html"
        ).read_text(encoding="utf-8")
        package_text = re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", package_page))
        platform = "macOS" if package["platform"] == "macos" else package["platform"].title()
        assert f"Package target {platform} {package['architecture']}" in package_text
        assert ">all<" not in package_page


def test_macos_package_present() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    stable = (
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    stable_packages = graph["manifests"]["stable"]["1.0.2"]["packages"]
    macos_packages = [
        package for package in stable_packages if package["kind"] == "macos_pkg"
    ]

    assert macos_packages
    for package in macos_packages:
        assert package["platform"] == "macos"
        assert package["name"].endswith(".pkg")
        assert package["name"] in stable
        assert package["url"] in stable


def test_macos_package_complete_binary_cohort() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        macos_packages = [
            package for package in manifest["packages"] if package["kind"] == "macos_pkg"
        ]

        assert len(macos_packages) == 1, channel
        package = macos_packages[0]
        assert package["platform"] == "macos"
        assert package["architecture"] == "arm64"

        binaries = {binary["name"]: binary for binary in package["binaries"]}
        assert set(binaries) == EXPECTED_BINARY_COHORT, f"{channel}:{package['name']}"

        package_page = (
            RELEASE_SITE_DIST
            / "channels"
            / channel
            / "packages"
            / package["id"]
            / "index.html"
        ).read_text(encoding="utf-8")

        for name, expected_path in EXPECTED_MACOS_BINARY_PATHS.items():
            binary = binaries[name]
            assert binary["version"] == package["version"], f"{channel}:{name}"
            assert binary["platform"] == "macos", f"{channel}:{name}"
            assert binary["architecture"] == "arm64", f"{channel}:{name}"
            assert binary["installed_path"] == expected_path, f"{channel}:{name}"
            assert binary["bytes"] > 0, f"{channel}:{name}"
            assert len(binary["digest"]["sha256"]) == 64, f"{channel}:{name}"
            assert len(binary["digest"]["blake3"]) == 64, f"{channel}:{name}"
            assert binary["sbom_component_ref"] == f"SPDXRef-File-{name}", (
                f"{channel}:{name}"
            )
            assert binary["installed_path"] in package_page
            assert binary["digest"]["sha256"][:8] + "..." in package_page
            assert binary["digest"]["blake3"][:8] + "..." in package_page
            assert f"<code>{binary['sbom_component_ref']}</code>" in package_page


def test_macos_package_binary_cohort() -> None:
    test_macos_package_complete_binary_cohort()


def test_linux_package_complete_binary_cohort() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    expected_architectures = {"arm64", "x86_64"}

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        linux_packages = [
            package
            for package in manifest["packages"]
            if package["kind"] == "debian_package"
        ]

        assert {package["architecture"] for package in linux_packages} == expected_architectures
        for package in linux_packages:
            assert package["platform"] == "linux"
            assert package["architecture"] in expected_architectures

            binaries = {binary["name"]: binary for binary in package["binaries"]}
            assert set(binaries) == EXPECTED_BINARY_COHORT, (
                f"{channel}:{package['name']}"
            )

            package_page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "packages"
                / package["id"]
                / "index.html"
            ).read_text(encoding="utf-8")

            for name, expected_path in EXPECTED_LINUX_BINARY_PATHS.items():
                binary = binaries[name]
                assert binary["version"] == package["version"], f"{channel}:{name}"
                assert binary["platform"] == "linux", f"{channel}:{name}"
                assert binary["architecture"] == package["architecture"], (
                    f"{channel}:{name}"
                )
                assert binary["installed_path"] == expected_path, f"{channel}:{name}"
                assert binary["bytes"] > 0, f"{channel}:{name}"
                assert len(binary["digest"]["sha256"]) == 64, f"{channel}:{name}"
                assert len(binary["digest"]["blake3"]) == 64, f"{channel}:{name}"
                assert binary["sbom_component_ref"] == f"SPDXRef-File-{name}", (
                    f"{channel}:{name}"
                )
                assert binary["installed_path"] in package_page
                assert binary["digest"]["sha256"][:8] + "..." in package_page
                assert binary["digest"]["blake3"][:8] + "..." in package_page
                assert f"<code>{binary['sbom_component_ref']}</code>" in package_page


def test_package_target_parity() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    expected_targets = {
        ("macos", "arm64", "macos_pkg"),
        ("linux", "arm64", "debian_package"),
        ("linux", "x86_64", "debian_package"),
    }

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        packages = manifest["packages"]
        observed_targets = {
            (package["platform"], package["architecture"], package["kind"])
            for package in packages
        }
        assert observed_targets == expected_targets, channel

        page = (
            RELEASE_SITE_DIST / "channels" / channel / "index.html"
        ).read_text(encoding="utf-8")
        packages_section = page.split("Capsem Packages", maxsplit=1)[1].split(
            "Profile References",
            maxsplit=1,
        )[0]
        for package in packages:
            platform = "macOS" if package["platform"] == "macos" else package["platform"].title()
            assert f"Package target {platform} {package['architecture']}" in packages_section
            assert package["name"] in packages_section
            assert package["url"] in packages_section


def test_full_binary_cohort() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for package in manifest["packages"]:
            binary_names = {binary["name"] for binary in package["binaries"]}
            assert binary_names == EXPECTED_BINARY_COHORT, f"{channel}:{package['name']}"


def test_package_detail_binary_cohort() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for package in manifest["packages"]:
            package_page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "packages"
                / package["id"]
                / "index.html"
            ).read_text(encoding="utf-8")
            binary_names = {binary["name"] for binary in package["binaries"]}

            assert binary_names == EXPECTED_BINARY_COHORT, f"{channel}:{package['name']}"
            for binary in package["binaries"]:
                assert binary["name"] in package_page
                assert binary["installed_path"] in package_page
                assert binary["version"] in package_page
                assert binary["bytes"] > 0, binary
                assert binary["digest"]["sha256"][:8] + "..." in package_page
                assert binary["digest"]["blake3"][:8] + "..." in package_page
                assert binary["sbom_component_ref"] in package_page


def test_owned_binary_cohort() -> None:
    test_full_binary_cohort()
    test_package_detail_binary_cohort()
    test_package_detail_is_binary_owner_view()
