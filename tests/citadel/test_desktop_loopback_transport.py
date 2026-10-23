"""WebKit can reach isolated loopback previews without disabling internet ATS."""

import copy
import json
import plistlib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
APP = ROOT / "crates/capsem-app"
RATIONALE = (
    "Per-VM previews use HTTP on isolated <exposure>.localhost origins. WebKit "
    "rejects their navigation with ATS error -1022 unless the app bundle allows "
    "localhost subdomains. Keep this exception local: never disable ATS globally "
    "or add internet domains. Tauri must merge the same canonical Info.plist."
)
TRANSPORT = {
    "NSExceptionDomains": {
        "localhost": {
            "NSIncludesSubdomains": True,
            "NSExceptionAllowsInsecureHTTPLoads": True,
        },
    },
}


def _scoped_transport(info: dict) -> bool:
    return info.get("NSAppTransportSecurity") == TRANSPORT


def test_native_app_allows_only_localhost_preview_transport() -> None:
    info = plistlib.loads((APP / "Info.plist").read_bytes())
    assert _scoped_transport(info), RATIONALE
    config = json.loads((APP / "tauri.conf.json").read_text())
    assert config["bundle"]["macOS"]["infoPlist"] == "Info.plist", RATIONALE


@pytest.mark.parametrize(
    "key",
    ["NSAllowsArbitraryLoads", "NSAllowsArbitraryLoadsInWebContent", "NSAllowsArbitraryLoadsForMedia"],
)
def test_global_transport_exceptions_are_refused(key: str) -> None:
    transport = {**copy.deepcopy(TRANSPORT), key: True}
    assert not _scoped_transport({"NSAppTransportSecurity": transport}), RATIONALE


def test_internet_domain_or_missing_subdomain_coverage_is_refused() -> None:
    transport = copy.deepcopy(TRANSPORT)
    transport["NSExceptionDomains"]["example.com"] = {}
    assert not _scoped_transport({"NSAppTransportSecurity": transport}), RATIONALE
    transport = copy.deepcopy(TRANSPORT)
    transport["NSExceptionDomains"]["localhost"]["NSIncludesSubdomains"] = False
    assert not _scoped_transport({"NSAppTransportSecurity": transport}), RATIONALE
