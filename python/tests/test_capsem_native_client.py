import unittest

from capsem_native_client import CapsemNativeClient, CapsemNativeError, parse_native_artifact


class CapsemNativeClientTest(unittest.TestCase):
    def test_client_routes_match_rust_native_surface(self):
        calls = []

        def transport(method, path, body):
            calls.append((method, path, body))
            if path == "/native/workspace/reset":
                return {"ok": True}
            if path == "/native/workspace/info":
                return {"workspaceId": "native-artifacts", "changeRequests": []}
            if path == "/native/workspace/snapshot":
                return {"workspaceId": "native-artifacts", "tail": []}
            if path == "/native/workspace/projection":
                return {"seq": 1, "elements": {}}
            if path == "/native/workspace/checkpoint":
                return {"checkpointSeq": 1, "workspaceId": "native-artifacts"}
            if path == "/native/workspace/select":
                return {"record": {"verb": "select", "target": body["target"]}, "deltas": []}
            if path == "/native/workspace/delete":
                return {"record": {"verb": "delete", "target": body["target"]}, "deltas": []}
            if path == "/native/workspace/title":
                return {"record": {"verb": "patch", "target": body["target"]}, "deltas": []}
            if path == "/native/workspace/change-request":
                return {"record": {"verb": "request", "target": body["target"]}, "deltas": []}
            if path == "/native/workspace/resolve":
                return {"record": {"verb": "respond", "target": body["taskId"]}, "deltas": []}
            if path == "/native/workspace/style":
                return {"record": {"verb": "patch", "target": body["target"]}, "deltas": []}
            if path == "/native/workspace/mutate":
                return {"record": {"verb": "patch", "target": body["target"]}, "deltas": []}
            if path == "/native/deck-proof":
                return {"ok": True, "summary": {"chartCount": 2}}
            if path == "/native/artifacts":
                return [valid_chart_artifact()]
            if path == "/native/artifacts/chart-revenue-by-quarter":
                return valid_chart_artifact()
            if path == "/native/telemetry":
                return [{"operation": "ui.chart"}]
            if path == "/native/mcp/tools":
                return {
                    "tools": [
                        {
                            "name": "local.ui.chart",
                            "adapter": "local__ui_chart",
                            "inputSchema": {"type": "object"},
                        }
                    ]
                }
            if path == "/native/data/sqlite/query":
                return {"ok": True, "rows": [{"quarter": "Q1"}]}
            if path == "/native/data/sheet":
                return {"id": body["id"], "kind": "sheet"}
            if path == "/native/generate/text":
                return {"id": body["id"], "kind": "generatedText"}
            if path == "/native/generate/image":
                return {"id": body["id"], "kind": "generatedImage"}
            if path == "/native/generate/embedding":
                return {"id": body["id"], "kind": "generatedEmbedding"}
            if path == "/native/ui/table":
                return {"id": body["id"], "kind": "table"}
            if path == "/native/ui/chart":
                return {"id": body["id"], "kind": "chart"}
            if path == "/native/ui/diagram":
                return {"id": body["id"], "kind": "diagram"}
            if path == "/native/ui/timeline":
                return {"id": body["id"], "kind": "timeline"}
            if path == "/native/ui/slide":
                return {"id": body["id"], "kind": "slide"}
            if path == "/native/ui/slide-deck":
                return {"id": body["id"], "kind": "slideDeck"}
            if path == "/native/ui/render-artifact":
                return {"ok": True, "component": "capsem-chart"}
            if path == "/native/export/spreadsheet":
                return {
                    "ok": True,
                    "artifactId": body["artifactId"],
                    "format": body["format"],
                    "status": "complete",
                    "fileUrl": "/native/export/files/sheet.xlsx",
                }
            if path == "/native/export/slide-deck":
                return {
                    "ok": False,
                    "artifactId": body["artifactId"],
                    "format": body["format"],
                    "status": "toolUnavailable",
                }
            raise AssertionError(f"unexpected path: {path}")

        client = CapsemNativeClient(transport=transport)

        self.assertTrue(client.native.reset_workspace()["ok"])
        self.assertTrue(client.workspace.reset()["ok"])
        self.assertEqual(client.workspace.info()["workspaceId"], "native-artifacts")
        self.assertEqual(client.workspace.snapshot()["tail"], [])
        self.assertEqual(client.workspace.projection()["seq"], 1)
        self.assertEqual(client.workspace.checkpoint()["checkpointSeq"], 1)
        self.assertEqual(client.workspace.select("artifact-1")["record"]["verb"], "select")
        self.assertEqual(
            client.workspace.comment(
                target="artifact-1",
                instruction="make it sharper",
                annotation={
                    "kind": "cardTitle",
                    "label": "Card title",
                    "path": ["card", "title"],
                },
            )["record"]["verb"],
            "request",
        )
        self.assertEqual(client.workspace.resolve("task-2")["record"]["verb"], "respond")
        self.assertEqual(
            client.workspace.title(target="artifact-1", title="New Title")["record"]["verb"],
            "patch",
        )
        self.assertEqual(
            client.workspace.mutate_title(target="artifact-1", title="Mutated Title")["record"]["verb"],
            "patch",
        )
        self.assertEqual(
            client.workspace.mutate_text(target="artifact-1", text="Mutated body")["record"]["verb"],
            "patch",
        )
        self.assertEqual(
            client.workspace.mutate_text(
                target="artifact-1",
                text="Mutated caption",
                selector="capsem-media[data-capsem-artifact-id='artifact-1'] >>> [data-capsem-node='artifact-1::caption']",
                host_selector="capsem-media[data-capsem-artifact-id='artifact-1']",
                shadow_selector="[data-capsem-node='artifact-1::caption']",
                source_request_seq=4,
            )["record"]["verb"],
            "patch",
        )
        self.assertEqual(
            client.workspace.style(
                target="artifact-1",
                selector="[data-capsem-role='title']",
                styles={"color": "var(--primary)"},
                source_request_seq=2,
            )["record"]["verb"],
            "patch",
        )
        self.assertEqual(
            client.workspace.mutate_style(
                target="artifact-1",
                selector="[data-capsem-role='body']",
                styles={"fontWeight": "700"},
                source_request_seq=3,
            )["record"]["verb"],
            "patch",
        )
        self.assertEqual(client.workspace.delete("artifact-1")["record"]["verb"], "delete")
        self.assertEqual(client.native.deck_proof()["summary"]["chartCount"], 2)
        self.assertEqual(client.native.artifacts()[0]["id"], "chart-revenue-by-quarter")
        self.assertEqual(client.native.artifact("chart-revenue-by-quarter")["id"], "chart-revenue-by-quarter")
        self.assertEqual(client.native.telemetry()[0]["operation"], "ui.chart")
        self.assertEqual(client.native.mcp_tools()["tools"][0]["name"], "local.ui.chart")
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
            client.generate.text(
                artifact_id="text-1",
                title="Text",
                prompt="generate text",
                system="be terse",
            )["kind"],
            "generatedText",
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
            client.generate.embedding(
                artifact_id="embedding-1",
                title="Embedding",
                input=["embed me"],
            )["kind"],
            "generatedEmbedding",
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
        self.assertEqual(
            client.ui.timeline(
                artifact_id="timeline-1",
                title="Timeline",
                lanes=[{"id": "build", "title": "Build"}],
                events=[
                    {
                        "id": "compile",
                        "title": "Compile",
                        "lane": "build",
                        "start": "2026-06-06",
                    }
                ],
            )["kind"],
            "timeline",
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
        self.assertTrue(client.export.spreadsheet("sheet-1")["ok"])
        self.assertEqual(client.export.slide_deck("deck-1")["status"], "toolUnavailable")

        self.assertEqual(
            calls[:21],
            [
                ("POST", "/native/workspace/reset", {}),
                ("POST", "/native/workspace/reset", {}),
                ("GET", "/native/workspace/info", None),
                ("GET", "/native/workspace/snapshot", None),
                ("GET", "/native/workspace/projection", None),
                ("POST", "/native/workspace/checkpoint", {}),
                ("POST", "/native/workspace/select", {"target": "artifact-1"}),
                (
                    "POST",
                    "/native/workspace/change-request",
                    {
                        "target": "artifact-1",
                        "instruction": "make it sharper",
                        "annotation": {
                            "kind": "cardTitle",
                            "label": "Card title",
                            "path": ["card", "title"],
                        },
                    },
                ),
                ("POST", "/native/workspace/resolve", {"taskId": "task-2"}),
                ("POST", "/native/workspace/title", {"target": "artifact-1", "title": "New Title"}),
                (
                    "POST",
                    "/native/workspace/mutate",
                    {"type": "title", "target": "artifact-1", "title": "Mutated Title"},
                ),
                (
                    "POST",
                    "/native/workspace/mutate",
                    {
                        "type": "text",
                        "target": "artifact-1",
                        "text": "Mutated body",
                        "selector": None,
                        "hostSelector": None,
                        "shadowSelector": None,
                        "sourceRequestSeq": None,
                    },
                ),
                (
                    "POST",
                    "/native/workspace/mutate",
                    {
                        "type": "text",
                        "target": "artifact-1",
                        "text": "Mutated caption",
                        "selector": "capsem-media[data-capsem-artifact-id='artifact-1'] >>> [data-capsem-node='artifact-1::caption']",
                        "hostSelector": "capsem-media[data-capsem-artifact-id='artifact-1']",
                        "shadowSelector": "[data-capsem-node='artifact-1::caption']",
                        "sourceRequestSeq": 4,
                    },
                ),
                (
                    "POST",
                    "/native/workspace/style",
                    {
                        "target": "artifact-1",
                        "selector": "[data-capsem-role='title']",
                        "hostSelector": None,
                        "shadowSelector": None,
                        "styles": {"color": "var(--primary)"},
                        "sourceRequestSeq": 2,
                    },
                ),
                (
                    "POST",
                    "/native/workspace/mutate",
                    {
                        "type": "style",
                        "target": "artifact-1",
                        "selector": "[data-capsem-role='body']",
                        "hostSelector": None,
                        "shadowSelector": None,
                        "styles": {"fontWeight": "700"},
                        "sourceRequestSeq": 3,
                    },
                ),
                ("POST", "/native/workspace/delete", {"target": "artifact-1"}),
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
                    "model": None,
                },
            ),
            calls,
        )
        self.assertIn(
            (
                "POST",
                "/native/generate/text",
                {
                    "id": "text-1",
                    "title": "Text",
                    "prompt": "generate text",
                    "system": "be terse",
                    "provider": "gemini",
                    "model": None,
                },
            ),
            calls,
        )
        self.assertIn(
            (
                "POST",
                "/native/generate/embedding",
                {
                    "id": "embedding-1",
                    "title": "Embedding",
                    "input": ["embed me"],
                    "provider": "openai",
                    "model": None,
                },
            ),
            calls,
        )
        self.assertIn(
            (
                "POST",
                "/native/ui/chart",
                {
                    "id": "chart-1",
                    "title": "Chart",
                    "chart": "barChart",
                    "sourceArtifact": "sheet-1",
                    "data": [{"house": "Compiler", "score": 1}],
                    "x": "house",
                    "series": [{"name": "score", "field": "score"}],
                    "xLabel": "House",
                    "xUnit": None,
                    "yLabel": "Score",
                    "yUnit": "score",
                    "stack": "none",
                    "direction": "vertical",
                    "legend": None,
                    "secondAxis": None,
                    "fit": None,
                    "export": ["png", "svg"],
                },
            ),
            calls,
        )
        self.assertIn(
            (
                "POST",
                "/native/ui/timeline",
                {
                    "id": "timeline-1",
                    "title": "Timeline",
                    "lanes": [{"id": "build", "title": "Build"}],
                    "events": [
                        {
                            "id": "compile",
                            "title": "Compile",
                            "lane": "build",
                            "start": "2026-06-06",
                        }
                    ],
                    "export": ["html"],
                },
            ),
            calls,
        )
        self.assertIn(
            ("POST", "/native/export/spreadsheet", {"artifactId": "sheet-1", "format": "xlsx"}),
            calls,
        )
        self.assertEqual(
            calls[-1],
            ("POST", "/native/export/slide-deck", {"artifactId": "deck-1", "format": "pptx"}),
        )

    def test_parse_native_artifact_rejects_unknown_chart_kind(self):
        artifact = valid_chart_artifact()
        artifact["spec"]["chart"] = "pieChart"

        with self.assertRaisesRegex(CapsemNativeError, "chart"):
            parse_native_artifact(artifact)

    def test_parse_native_artifact_rejects_incompatible_chart_options(self):
        artifact = valid_chart_artifact()
        artifact["spec"]["chart"] = "lineChart"
        artifact["spec"]["stack"] = "stacked"

        with self.assertRaisesRegex(CapsemNativeError, "lineChart does not support stack"):
            parse_native_artifact(artifact)


def valid_chart_artifact():
    return {
        "id": "chart-revenue-by-quarter",
        "kind": "chart",
        "title": "Revenue By Quarter",
        "handle": "capsem://artifact/abcd",
        "spec": {
            "component": "capsem-chart",
            "chart": "barChart",
            "sourceArtifact": "sheet-revenue",
            "data": [{"quarter": "Q1", "revenue": 10}],
            "x": "quarter",
            "series": [{"name": "revenue", "field": "revenue"}],
            "xLabel": "Quarter",
            "yLabel": "Revenue",
            "yUnit": "usd",
            "stack": "none",
            "direction": "vertical",
            "export": ["png", "svg"],
        },
    }


if __name__ == "__main__":
    unittest.main()
