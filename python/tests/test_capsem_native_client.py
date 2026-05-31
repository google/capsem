import unittest

from capsem_native_client import CapsemNativeClient


class CapsemNativeClientTest(unittest.TestCase):
    def test_client_routes_match_rust_native_surface(self):
        calls = []

        def transport(method, path, body):
            calls.append((method, path, body))
            if path == "/native/deck-proof":
                return {"ok": True, "summary": {"chartCount": 2}}
            if path == "/native/artifacts":
                return [{"id": "chart-revenue-by-quarter"}]
            if path == "/native/artifacts/chart-revenue-by-quarter":
                return {"id": "chart-revenue-by-quarter"}
            if path == "/native/data/sqlite/query":
                return {"ok": True, "rows": [{"quarter": "Q1"}]}
            if path == "/native/ui/render-artifact":
                return {"ok": True, "component": "capsem-chart"}
            raise AssertionError(f"unexpected path: {path}")

        client = CapsemNativeClient(transport=transport)

        self.assertEqual(client.native.deck_proof()["summary"]["chartCount"], 2)
        self.assertEqual(client.native.artifacts()[0]["id"], "chart-revenue-by-quarter")
        self.assertEqual(client.native.artifact("chart-revenue-by-quarter")["id"], "chart-revenue-by-quarter")
        self.assertEqual(client.data.sqlite.query("select 1")["rows"][0]["quarter"], "Q1")
        self.assertEqual(client.ui.render_artifact("chart-revenue-by-quarter")["component"], "capsem-chart")

        self.assertEqual(
            calls,
            [
                ("GET", "/native/deck-proof", None),
                ("GET", "/native/artifacts", None),
                ("GET", "/native/artifacts/chart-revenue-by-quarter", None),
                ("POST", "/native/data/sqlite/query", {"sql": "select 1"}),
                ("POST", "/native/ui/render-artifact", {"artifactId": "chart-revenue-by-quarter"}),
            ],
        )


if __name__ == "__main__":
    unittest.main()
