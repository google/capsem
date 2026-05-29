import { installPlugin, runPlugin } from "../src/component-runner.js";

const source = `
export function invoke(objectJson, contextJson) {
  const object = JSON.parse(objectJson);
  const context = JSON.parse(contextJson);
  object.labels = context.trace?.labels ?? [];
  object.called = "invoke";
  return JSON.stringify(object);
}

export function inspect(objectJson, contextJson) {
  const object = JSON.parse(objectJson);
  const context = JSON.parse(contextJson);
  return JSON.stringify({
    kind: "Inspection",
    object_kind: object.kind,
    labels: context.trace?.labels ?? [],
    called: "inspect"
  });
}
`;

const iterations = Number(process.env.N || "3");
const install = await installPlugin({
  source,
  manifest: {
    id: "bench.security",
    name: "Bench Security Plugin",
    version: "0.1.0",
    callbacks: ["invoke", "inspect"],
    capabilities: [],
  },
});
console.log(JSON.stringify({ phase: "install", result: install }));
if (!install.ok) {
  process.exit(1);
}

for (let i = 0; i < iterations; i += 1) {
  const result = await runPlugin({
    plugin_id: install.plugin_id,
    object: { kind: "ModelOutput", content: { text: "hi" } },
    context: { trace: { labels: ["bench"] } },
    functions: ["invoke", "inspect"],
    runs: 3,
  });
  console.log(JSON.stringify({ iteration: i + 1, result }));
}
