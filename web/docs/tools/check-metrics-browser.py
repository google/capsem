"""Exercise the built metrics pages on desktop and mobile without running benchmarks."""

import argparse
import functools
import http.server
import threading
from pathlib import Path

from playwright.sync_api import sync_playwright

parser = argparse.ArgumentParser()
parser.add_argument("--base-url")
parser.add_argument(
    "--output", type=Path, default=Path("cache/target/tests/docs-metrics")
)
args = parser.parse_args()
server = None
if not args.base_url:

    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(
        ("127.0.0.1", 0), functools.partial(Quiet, directory="web/docs/dist")
    )
    threading.Thread(target=server.serve_forever, daemon=True).start()
    args.base_url = f"http://127.0.0.1:{server.server_port}"
out = args.output
out.mkdir(parents=True, exist_ok=True)
try:
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for width in [1440, 390]:
            page = browser.new_page(
                viewport={"width": width, "height": 1000}, reduced_motion="reduce"
            )
            errors = []
            page.on("pageerror", lambda e, collected=errors: collected.append(str(e)))
            page.goto(args.base_url + "/architecture/statistics/")
            radar = page.locator('metric-chart[data-group="composition"]').first
            radar.scroll_into_view_if_needed()
            radar.locator("svg").wait_for()
            pie = page.locator('metric-chart[data-group="composition"]').nth(1)
            pie.scroll_into_view_if_needed()
            pie.locator("svg").wait_for()
            radar.scroll_into_view_if_needed()
            assert (
                page.locator('metric-chart[data-group="composition"] svg').count() == 2
            )
            radar.locator("select").select_option("0")
            assert (
                page.locator('metric-chart[data-group="composition"]')
                .nth(1)
                .locator("select")
                .input_value()
                == "0"
            )
            assert "1,020" in radar.locator("[data-chart-note]").inner_text()
            radar.locator("select").select_option(
                index=radar.locator("option").count() - 1
            )
            for kind in ["csv", "svg"]:
                with page.expect_download() as download:
                    radar.locator(f'[data-export="{kind}"]').click()
                path = out / f"{width}-{download.value.suggested_filename}"
                download.value.save_as(path)
                assert path.stat().st_size > 30
            page.screenshot(path=str(out / f"statistics-{width}.png"))
            dep = page.locator("crate-dependencies")
            dep.scroll_into_view_if_needed()
            dep.locator("[data-month]").select_option("0")
            assert dep.locator("joint-diagram").is_hidden()
            assert "No Rust" in dep.locator("[data-status]").inner_text()
            dep.locator("[data-month]").select_option(
                index=dep.locator("[data-month] option").count() - 1
            )
            dep.locator("[data-focus]").fill("capsem-sdk")
            dep.locator(".diagram-paper svg").wait_for()
            assert "crates" in dep.locator("[data-status]").inner_text()
            dep.locator("[data-kind]").select_option("all")
            dep.locator("[data-all]").click()
            dep.get_by_role("button", name="Zoom in", exact=True).click()
            dep.get_by_role("button", name="Fit", exact=True).click()
            page.screenshot(path=str(out / f"dependencies-{width}.png"))
            assert page.evaluate(
                "document.documentElement.scrollWidth <= innerWidth"
            ), "Statistics overflow"
            page.goto(args.base_url + "/architecture/benchmarks/")
            first = page.locator("metric-chart").first
            first.scroll_into_view_if_needed()
            first.locator("svg").wait_for()
            assert (
                "Latest retained regular recording"
                in page.locator(".metric-meta").inner_text()
            )
            first.locator("select").select_option(label="Forked image size")
            assert "MiB" in first.locator("[data-table]").text_content()
            theme = page.locator("starlight-theme-select select").first
            theme.select_option("dark", force=True)
            assert page.locator("html").get_attribute("data-theme") == "dark"
            theme.select_option("light", force=True)
            with page.expect_download() as raw:
                page.get_by_role(
                    "link", name="Download recorded JSON", exact=True
                ).click()
            assert raw.value.suggested_filename == "benchmark-records.json"
            page.screenshot(path=str(out / f"benchmarks-{width}.png"))
            assert page.evaluate(
                "document.documentElement.scrollWidth <= innerWidth"
            ), "Benchmarks overflow"
            assert not errors, errors
            print(
                f"{width}px: synchronized charts, CSV/SVG downloads, dependency filtering/zoom, benchmark selections, no overflow or browser errors"
            )
            page.close()
        browser.close()
finally:
    if server:
        server.shutdown()
