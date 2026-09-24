import { describe, expect, it } from "vitest";

import {
  applyRenderDeltas,
  checkpointWorkspace,
  mutateWorkspaceStyle,
  mutateWorkspaceText,
  mutateWorkspaceTextPatch,
  mutateWorkspaceTitle,
  parseNativeArtifact,
  parseWorkspaceSnapshot,
  parseWorkspaceStreamMessage,
  projectionToMaps,
  reportRenderError,
  requestWorkspaceChange,
  resolveWorkspaceTask,
  selectWorkspaceElement,
  type NativeArtifact,
  type RenderElement,
} from "../ui-preview/src/nativeDeck.js";

const artifact: NativeArtifact = {
  id: "generated-image-demo",
  kind: "generatedImage",
  title: "Generated Image Demo",
  handle: "capsem://artifact/demo",
  spec: {
    component: "capsem-media",
    media: "image",
    provider: "gemini",
    model: null,
    prompt: "demo image",
    revisedPrompt: null,
    usage: null,
    cost: null,
    durationMs: null,
    status: "generated",
    dataUrl: "data:image/png;base64,AAAA",
  },
};

const element: RenderElement = {
  id: artifact.id,
  title: artifact.title,
  contentType: "artifact",
  status: "complete",
  component: "capsem-media",
  provenance: {
    createdSeq: 1,
    createdAt: "1",
    createdBy: "local.generate.image",
    updatedSeq: 1,
    updatedAt: "1",
    updatedBy: "local.generate.image",
    lastRecordId: "rec-1",
    lastVerb: "create",
  },
  artifact,
};

const task = {
  id: "task-2",
  target: artifact.id,
  instruction: "make the title yellow",
  annotation: {
    kind: "cardTitle",
    label: "Card title",
    path: ["card", "title"],
    selector: `[data-capsem-element-id="${artifact.id}"] [data-capsem-role="card-title"]`,
    selectorVerified: true,
  },
  status: "open" as const,
  createdSeq: 2,
  createdAt: "2",
  createdBy: "chat.ui",
  updatedSeq: 2,
  updatedAt: "2",
  updatedBy: "chat.ui",
  sourceRecordId: "rec-2",
};

describe("native workspace stream contract", () => {
  it("parses snapshot projection into keyed maps", () => {
    const snapshot = parseWorkspaceSnapshot({
      workspaceId: "native-artifacts",
      checkpoint: {
        checkpointSeq: 1,
        createdAt: "1",
        workspaceId: "native-artifacts",
        projectionVersion: 1,
        recordsCompacted: { start: 1, end: 1 },
        projection: {
          seq: 1,
          elements: { [artifact.id]: element },
          topology: {
            roots: [artifact.id],
            nodes: {
              [artifact.id]: { id: artifact.id, slot: "chat", index: 0 },
            },
          },
          selected: artifact.id,
          tasks: { [task.id]: task },
        },
      },
      projection: {
        seq: 1,
        elements: { [artifact.id]: element },
        topology: {
          roots: [artifact.id],
          nodes: {
            [artifact.id]: { id: artifact.id, slot: "chat", index: 0 },
          },
        },
        selected: artifact.id,
        tasks: { [task.id]: task },
      },
      tail: [],
    });

    const maps = projectionToMaps(snapshot.projection);
    expect(maps.elementsById.get(artifact.id)?.component).toBe("capsem-media");
    expect(maps.topology.roots).toEqual([artifact.id]);
    expect(maps.topology.nodes[artifact.id]?.slot).toBe("chat");
    expect(maps.tasksById.get(task.id)?.instruction).toBe("make the title yellow");
    expect(snapshot.checkpoint.projection.tasks?.[task.id]?.annotation?.selectorVerified).toBe(true);
  });

  it("applies Rust-emitted deltas without deriving UI behavior", () => {
    const next = applyRenderDeltas(new Map(), { roots: [], nodes: {} }, "", [
      { type: "upsertElement", id: artifact.id, element },
      { type: "upsertTopologyNode", node: { id: artifact.id, slot: "chat", index: 0 } },
      { type: "upsertTask", id: task.id, task },
      { type: "select", id: artifact.id },
    ]);

    expect(next.elementsById.get(artifact.id)?.artifact?.title).toBe("Generated Image Demo");
    expect(next.topology.roots).toEqual([artifact.id]);
    expect(next.tasksById.get(task.id)?.status).toBe("open");
    expect(next.selectedId).toBe(artifact.id);
  });

  it("applies select and delete deltas against keyed maps", () => {
    const first = { ...element, id: "generated-image-first", title: "First" };
    const second = {
      ...element,
      id: "generated-image-second",
      title: "Second",
      artifact: { ...artifact, id: "generated-image-second", title: "Second" },
    };
    const seeded = applyRenderDeltas(new Map(), { roots: [], nodes: {} }, "", [
      { type: "upsertElement", id: first.id, element: first },
      { type: "upsertElement", id: second.id, element: second },
      { type: "upsertTopologyNode", node: { id: first.id, slot: "chat", index: 0 } },
      { type: "upsertTopologyNode", node: { id: second.id, slot: "chat", index: 1 } },
      { type: "select", id: second.id },
    ]);

    expect(seeded.selectedId).toBe(second.id);

    const deleted = applyRenderDeltas(seeded.elementsById, seeded.topology, seeded.selectedId, [
      { type: "deleteElement", id: second.id },
      { type: "deleteTopologyNode", id: second.id },
      { type: "select", id: first.id },
    ]);

    expect(deleted.elementsById.has(second.id)).toBe(false);
    expect(deleted.topology.roots).toEqual([first.id]);
    expect(deleted.selectedId).toBe(first.id);
  });

  it("applies title-only patches without changing artifact payload", () => {
    const patched = {
      ...element,
      title: "Live Edited Card Title",
      provenance: {
        ...element.provenance,
        updatedSeq: 2,
        updatedAt: "2",
        updatedBy: "local.workspace.title",
        lastRecordId: "rec-2",
        lastVerb: "patch" as const,
      },
      artifact: { ...artifact, title: "Generated Image Demo" },
    };
    const next = applyRenderDeltas(
      new Map([[artifact.id, element]]),
      {
        roots: [artifact.id],
        nodes: {
          [artifact.id]: { id: artifact.id, slot: "chat", index: 0 },
        },
      },
      artifact.id,
      [{ type: "upsertElement", id: artifact.id, element: patched }],
    );

    const updated = next.elementsById.get(artifact.id);
    expect(updated?.title).toBe("Live Edited Card Title");
    expect(updated?.artifact?.title).toBe("Generated Image Demo");
    expect(updated?.provenance.createdBy).toBe("local.generate.image");
    expect(updated?.provenance.updatedBy).toBe("local.workspace.title");
    expect(next.topology.roots).toEqual([artifact.id]);
  });

  it("rejects malformed stream messages", () => {
    expect(() => parseWorkspaceStreamMessage({ type: "record", frame: { deltas: [] } })).toThrow(
      /record/,
    );
    expect(() => parseWorkspaceSnapshot({ workspaceId: "x" })).toThrow(/projection/);
  });

  it("rejects malformed native artifact specs before render", () => {
    expect(() =>
      parseNativeArtifact({
        id: "chart-bad",
        kind: "chart",
        title: "Bad Chart",
        handle: "capsem://artifact/bad",
        spec: {
          component: "capsem-chart",
          chart: "pieChart",
          sourceArtifact: "sheet-test",
          data: [],
          x: "house",
          series: [{ name: "score", field: "score" }],
          xLabel: "House",
          yLabel: "Score",
          yUnit: "score",
        },
      }),
    ).toThrow(/chart\.chart/);

    expect(() =>
      parseNativeArtifact({
        id: "chart-stacked-line",
        kind: "chart",
        title: "Stacked Line",
        handle: "capsem://artifact/bad",
        spec: {
          component: "capsem-chart",
          chart: "lineChart",
          sourceArtifact: "sheet-test",
          data: [],
          x: "house",
          series: [{ name: "score", field: "score" }],
          xLabel: "House",
          yLabel: "Score",
          yUnit: "score",
          stack: "stacked",
        },
      }),
    ).toThrow(/lineChart does not support stack mode/);
  });

  it("posts selection and change requests through the native workspace API", async () => {
    const calls: Array<{ url: string; body: unknown }> = [];
    const fetcher = async (url: string | URL | Request, init?: RequestInit) => {
      calls.push({
        url: String(url),
        body: init?.body ? JSON.parse(String(init.body)) : undefined,
      });
      return new Response(
        JSON.stringify({
          record: {
            seq: calls.length,
            id: `rec-${calls.length}`,
            timestamp: "1",
            role: calls.length === 1 ? "tool" : "user",
            principal: calls.length === 1 ? "local.workspace.select" : "chat.ui",
            title: calls.length === 1 ? `Select ${artifact.id}` : `Change ${artifact.id}`,
            contentType: calls.length === 1 ? "status" : "action",
            verb: calls.length === 1 ? "select" : "request",
            status: calls.length === 1 ? "complete" : "pending",
            target: artifact.id,
            content:
              calls.length === 1
                ? { type: "status", message: `Select ${artifact.id}` }
                : {
                    type: "action",
                    name: "ui.change",
                    payload: {
                      instruction: "make it blue",
                      annotation: {
                        kind: "cardTitle",
                        label: "Card title",
                        path: ["card", "title"],
                      },
                    },
                  },
          },
          deltas: calls.length === 1 ? [{ type: "select", id: artifact.id }] : [],
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    };

    const selected = await selectWorkspaceElement(artifact.id, fetcher as typeof fetch);
    const requested = await requestWorkspaceChange(
      artifact.id,
      "make it blue",
      {
        kind: "cardTitle",
        label: "Card title",
        path: ["card", "title"],
      },
      fetcher as typeof fetch,
    );

    expect(calls).toEqual([
      { url: "/native/workspace/select", body: { target: artifact.id } },
      {
        url: "/native/workspace/change-request",
        body: {
          target: artifact.id,
          instruction: "make it blue",
          annotation: {
            kind: "cardTitle",
            label: "Card title",
            path: ["card", "title"],
          },
        },
      },
    ]);
    expect(selected.deltas).toEqual([{ type: "select", id: artifact.id }]);
    expect(requested.record.verb).toBe("request");
    expect(requested.record.content).toEqual({
      type: "action",
      name: "ui.change",
      payload: {
        instruction: "make it blue",
        annotation: {
          kind: "cardTitle",
          label: "Card title",
          path: ["card", "title"],
        },
      },
    });
  });

  it("posts workspace checkpoint through the native workspace API", async () => {
    const calls: Array<{ url: string; method: string | undefined }> = [];
    const fetcher = async (url: string | URL | Request, init?: RequestInit) => {
      calls.push({ url: String(url), method: init?.method });
      return new Response(
        JSON.stringify({
          checkpointSeq: 2,
          createdAt: "2",
          workspaceId: "native-artifacts",
          projectionVersion: 1,
          recordsCompacted: { start: 1, end: 2 },
          projection: {
            seq: 2,
            elements: { [artifact.id]: element },
            topology: {
              roots: [artifact.id],
              nodes: {
                [artifact.id]: { id: artifact.id, slot: "chat", index: 0 },
              },
            },
            tasks: { [task.id]: task },
            selected: artifact.id,
          },
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    };

    const checkpoint = await checkpointWorkspace(fetcher as typeof fetch);

    expect(calls).toEqual([{ url: "/native/workspace/checkpoint", method: "POST" }]);
    expect(checkpoint.checkpointSeq).toBe(2);
    expect(checkpoint.projection.tasks?.[task.id]?.status).toBe("open");
  });

  it("posts task resolution through the native workspace API", async () => {
    const resolvedTask = {
      ...task,
      status: "resolved" as const,
      updatedSeq: 3,
      updatedAt: "3",
      updatedBy: "assistant.ui",
      resolvedSeq: 3,
      resolvedAt: "3",
      resolvedBy: "assistant.ui",
    };
    const calls: Array<{ url: string; body: unknown }> = [];
    const fetcher = async (url: string | URL | Request, init?: RequestInit) => {
      calls.push({
        url: String(url),
        body: init?.body ? JSON.parse(String(init.body)) : undefined,
      });
      return new Response(
        JSON.stringify({
          record: {
            seq: 3,
            id: "rec-3",
            timestamp: "3",
            role: "assistant",
            principal: "assistant.ui",
            title: `Resolve ${task.id}`,
            contentType: "action",
            verb: "respond",
            status: "complete",
            target: task.id,
            content: {
              type: "action",
              name: "ui.resolve",
              payload: { taskId: task.id },
            },
          },
          deltas: [{ type: "upsertTask", id: task.id, task: resolvedTask }],
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    };

    const frame = await resolveWorkspaceTask(task.id, fetcher as typeof fetch);

    expect(calls).toEqual([
      { url: "/native/workspace/resolve", body: { taskId: task.id } },
    ]);
    expect(frame.record.verb).toBe("respond");
    expect(frame.deltas).toEqual([{ type: "upsertTask", id: task.id, task: resolvedTask }]);
  });

  it("posts canonical workspace mutations through the native workspace API", async () => {
    const calls: Array<{ url: string; body: unknown }> = [];
    const fetcher = async (url: string | URL | Request, init?: RequestInit) => {
      const body = init?.body ? JSON.parse(String(init.body)) : undefined;
      calls.push({ url: String(url), body });
      return new Response(
        JSON.stringify({
          record: {
            seq: calls.length + 3,
            id: `rec-${calls.length + 3}`,
            timestamp: "4",
            role: "tool",
            principal: "local.ui.mutate",
            title: `Patch ${artifact.id}`,
            contentType: "elementPatch",
            verb: "patch",
            status: "complete",
            target: artifact.id,
            content: {
              type: "elementPatch",
              title: body.type === "title" ? body.title : undefined,
              stylePatches: body.type === "style" ? [body] : [],
            },
          },
          deltas: [{ type: "upsertElement", id: artifact.id, element }],
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    };

    const titleFrame = await mutateWorkspaceTitle(
      artifact.id,
      "Sharper title",
      fetcher as typeof fetch,
    );
    const styleFrame = await mutateWorkspaceStyle(
      {
        target: artifact.id,
        selector: "[data-capsem-role='title']",
        styles: { color: "var(--primary)" },
        sourceRequestSeq: 2,
      },
      fetcher as typeof fetch,
    );
    const textFrame = await mutateWorkspaceText(
      artifact.id,
      "Sharper body",
      fetcher as typeof fetch,
    );
    const textPatchFrame = await mutateWorkspaceTextPatch(
      {
        target: artifact.id,
        text: "Sharper caption",
        selector: 'capsem-media[data-capsem-artifact-id="artifact-1"] >>> [data-capsem-node="artifact-1::caption"]',
        hostSelector: 'capsem-media[data-capsem-artifact-id="artifact-1"]',
        shadowSelector: '[data-capsem-node="artifact-1::caption"]',
        sourceRequestSeq: 7,
      },
      fetcher as typeof fetch,
    );

    expect(calls).toEqual([
      {
        url: "/native/workspace/mutate",
        body: { type: "title", target: artifact.id, title: "Sharper title" },
      },
      {
        url: "/native/workspace/mutate",
        body: {
          type: "style",
          target: artifact.id,
          selector: "[data-capsem-role='title']",
          styles: { color: "var(--primary)" },
          sourceRequestSeq: 2,
        },
      },
      {
        url: "/native/workspace/mutate",
        body: { type: "text", target: artifact.id, text: "Sharper body" },
      },
      {
        url: "/native/workspace/mutate",
        body: {
          type: "text",
          target: artifact.id,
          text: "Sharper caption",
          selector: 'capsem-media[data-capsem-artifact-id="artifact-1"] >>> [data-capsem-node="artifact-1::caption"]',
          hostSelector: 'capsem-media[data-capsem-artifact-id="artifact-1"]',
          shadowSelector: '[data-capsem-node="artifact-1::caption"]',
          sourceRequestSeq: 7,
        },
      },
    ]);
    expect(titleFrame.record.verb).toBe("patch");
    expect(styleFrame.record.principal).toBe("local.ui.mutate");
    expect(textFrame.record.principal).toBe("local.ui.mutate");
    expect(textPatchFrame.record.principal).toBe("local.ui.mutate");
  });

  it("reports render errors through the native render telemetry API", async () => {
    const calls: Array<{ url: string; body: unknown }> = [];
    const fetcher = async (url: string | URL | Request, init?: RequestInit) => {
      const body = init?.body ? JSON.parse(String(init.body)) : undefined;
      calls.push({ url: String(url), body });
      return new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    };

    await reportRenderError(
      {
        message: "plotly render failed",
        source: "browser",
        status: "failed",
        artifactId: "chart-demo",
        component: "capsem-chart",
        renderer: "plotly",
        phase: "componentRender",
      },
      fetcher as typeof fetch,
    );

    expect(calls).toEqual([
      {
        url: "/native/ui/render-error",
        body: {
          message: "plotly render failed",
          source: "browser",
          status: "failed",
          artifactId: "chart-demo",
          component: "capsem-chart",
          renderer: "plotly",
          phase: "componentRender",
        },
      },
    ]);
  });
});
