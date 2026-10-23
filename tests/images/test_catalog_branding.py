"""Published catalog icons are complete, bundled passive images, not trackers."""

import struct
import tomllib
import xml.etree.ElementTree as ET
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[2]
PUBLIC = ROOT / "web/app/public"


def test_supported_image_branding_resolves_to_safe_bundled_assets() -> None:
    images = tomllib.loads((ROOT / "images/catalog.toml").read_text())["images"]
    for image, metadata in images.items():
        assert metadata.get("publisher"), f"{image} needs its real publisher"
        url = urlsplit(metadata.get("url", ""))
        assert url.scheme == "https" and url.netloc and not url.username and not url.password
        icon = metadata.get("icon", "")
        assert icon.startswith("/icons/products/") and ".." not in icon, image
        path = PUBLIC / icon.removeprefix("/")
        assert path.is_file(), f"{image}: catalog icon is not in the embedded public bundle"
        content = path.read_bytes()
        assert len(content) < 64 * 1024, "small catalog icons must not fetch megabyte artwork"
        if path.suffix == ".svg":
            assert b"<!DOCTYPE" not in content and b"<!ENTITY" not in content
            svg = ET.fromstring(content)
            assert svg.tag == "{http://www.w3.org/2000/svg}svg"
            for node in svg.iter():
                tag = node.tag.split("}")[-1]
                assert tag not in {"script", "foreignObject", "image", "iframe", "animate", "set"}
                for name, value in node.attrib.items():
                    assert not name.lower().startswith("on"), "icons have no event handlers"
                    if name.split("}")[-1] == "href":
                        assert value.startswith("#"), "SVG references stay inside the image"
            assert b"https://" not in content and b"http://" not in content.replace(
                b"http://www.w3.org/2000/svg", b""
            ).replace(b"http://www.w3.org/1999/xlink", b"")
        else:
            assert path.suffix == ".png" and content.startswith(b"\x89PNG\r\n\x1a\n")
            width, height = struct.unpack(">II", content[16:24])
            assert 32 <= width <= 1024 and 32 <= height <= 1024
