import unittest

from capsem_native_client import CapsemNativeClient


class CapsemNativeClientTest(unittest.TestCase):
    def test_client_routes_match_rust_native_surface(self):
        calls = []

        def transport(method, path, body):
            calls.append((method, path, body))
            if path == "/native/workspace/reset":
                return {"ok": True}
            if path == "/native/deck-proof":
                return {"ok": True, "summary": {"chartCount": 2}}
            if path == "/native/artifacts":
                return [{"id": "chart-revenue-by-quarter"}]
            if path == "/native/artifacts/chart-revenue-by-quarter":
                return {"id": "chart-revenue-by-quarter"}
            if path == "/native/telemetry":
                return [{"operation": "ui.chart"}]
            if path == "/native/mcp/tools":
                return {"tools": [{"name": "local__ui_chart"}]}
            if path == "/native/data/sqlite/query":
                return {"ok": True, "rows": [{"quarter": "Q1"}]}
            if path == "/native/data/sheet":
                return {"id": body["id"], "kind": "sheet"}
            if path == "/native/generate/image":
                return {"id": body["id"], "kind": "generatedImage"}
            if path == "/native/ui/table":
                return {"id": body["id"], "kind": "table"}
            if path == "/native/ui/chart":
                return {"id": body["id"], "kind": "chart"}
            if path == "/native/ui/diagram":
                return {"id": body["id"], "kind": "diagram"}
            if path == "/native/ui/slide":
                return {"id": body["id"], "kind": "slide"}
            if path == "/native/ui/slide-deck":
                return {"id": body["id"], "kind": "slideDeck"}
            if path == "/native/ui/render-artifact":
                return {"ok": True, "component": "capsem-chart"}
            raise AssertionError(f"unexpected path: {path}")

        client = CapsemNativeClient(transport=transport)

        self.assertTrue(client.native.reset_workspace()["ok"])
        self.assertEqual(client.native.deck_proof()["summary"]["chartCount"], 2)
        self.assertEqual(client.native.artifacts()[0]["id"], "chart-revenue-by-quarter")
        self.assertEqual(client.native.artifact("chart-revenue-by-quarter")["id"], "chart-revenue-by-quarter")
        self.assertEqual(client.native.telemetry()[0]["operation"], "ui.chart")
        self.assertEqual(client.native.mcp_tools()["tools"][0]["name"], "local__ui_chart")
        self.assertEqual(client.data.sqlite.query("select 1")["rows"][0]["quarter"], "Q1")
        self.assertEqual(
            client.ui.sheet(
                artifact_id="sheet-1",
                title="Sheet",
                columns=["house"],
                rows=[{"house": "Compiler"}],
            )["kind"],
            "sheet",
        )
        self.assertEqual(
            client.generate.image(
                artifact_id="image-1",
                title="Image",
                prompt="generate image",
            )["kind"],
            "generatedImage",
        )
        self.assertEqual(
            client.ui.table(
                artifact_id="table-1",
                title="Table",
                source_artifact="sheet-1",
                columns=["house"],
                rows=[{"house": "Compiler"}],
            )["kind"],
            "table",
        )
        self.assertEqual(
            client.ui.chart(
                artifact_id="chart-1",
                title="Chart",
                chart="barChart",
                source_artifact="sheet-1",
                data=[{"house": "Compiler", "score": 1}],
                x="house",
                series=[{"name": "score", "field": "score"}],
                x_label="House",
                y_label="Score",
                y_unit="score",
            )["kind"],
            "chart",
        )
        self.assertEqual(
            client.ui.diagram(
                artifact_id="diagram-1",
                title="Diagram",
                source="flowchart LR\n  A --> B",
            )["kind"],
            "diagram",
        )
        slide = client.ui.slide(
            artifact_id="slide-1",
            title="Slide",
            blocks=[{"kind": "text", "title": "Hello", "body": "World"}],
        )
        self.assertEqual(slide["kind"], "slide")
        self.assertEqual(
            client.ui.slide_deck(
                artifact_id="deck-1",
                title="Deck",
                slides=[{"artifactId": "slide-1", "title": "Slide"}],
            )["kind"],
            "slideDeck",
        )
        self.assertEqual(client.ui.render_artifact("chart-revenue-by-quarter")["component"], "capsem-chart")

        self.assertEqual(
            calls[:6],
            [
                ("POST", "/native/workspace/reset", {}),
                ("GET", "/native/deck-proof", None),
                ("GET", "/native/artifacts", None),
                ("GET", "/native/artifacts/chart-revenue-by-quarter", None),
                ("GET", "/native/telemetry", None),
                ("GET", "/native/mcp/tools", None),
            ],
        )
        self.assertIn(("POST", "/native/data/sqlite/query", {"sql": "select 1"}), calls)
        self.assertIn(
            (
                "POST",
                "/native/generate/image",
                {
                    "id": "image-1",
                    "title": "Image",
                    "prompt": "generate image",
                    "provider": "gemini",
                },
            ),
            calls,
        )
        self.assertEqual(calls[-1], ("POST", "/native/ui/render-artifact", {"artifactId": "chart-revenue-by-quarter"}))


if __name__ == "__main__":
    unittest.main()
