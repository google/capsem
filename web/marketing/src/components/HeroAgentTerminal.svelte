<script lang="ts">
  import { onMount } from "svelte";
  import type { Terminal } from "@xterm/xterm";
  import HeroCodexChrome from "./HeroCodexChrome.svelte";
  let { content, input, agent, directory }: { content: string; input: string | null; agent: "agy" | "codex" | "shell"; directory: string } = $props();
  let element: HTMLDivElement;
  let terminal: Terminal | undefined;
  let ready = $state(false);
  let previous = "";
  $effect(() => {
    const text = content + (agent !== "codex" ? input ?? "" : "");
    if (!ready || !terminal) return;
    if (text === previous) return;
    const line = text.lastIndexOf("\r\n");
    const oldLine = previous.lastIndexOf("\r\n");
    const spinnerOnly = line >= 0 && oldLine >= 0 && text.slice(0, line) === previous.slice(0, oldLine);
    terminal.write(text.startsWith(previous) ? text.slice(previous.length) : spinnerOnly ? `\r\x1b[2K${text.slice(line + 2)}` : `\x1b[?25l\x1b[2J\x1b[H${text}`);
    previous = text;
  });
  onMount(() => {
    let disposed = false;
    let cleanup = () => {};
    void (async () => {
      const [{ Terminal }, { FitAddon }] = await Promise.all([import("@xterm/xterm"), import("@xterm/addon-fit")]);
      if (disposed) return;
      const style = getComputedStyle(element);
      terminal = new Terminal({ disableStdin: true, cursorBlink: false, cursorStyle: "block", fontFamily: '"SFMono-Regular", Menlo, Consolas, monospace', fontSize: element.clientWidth < 420 ? 11 : 13, lineHeight: 1.35, convertEol: true, theme: { background: style.getPropertyValue("--color-demo-tui").trim(), foreground: style.getPropertyValue("--color-demo-text").trim() } });
      const fit = new FitAddon(); terminal.loadAddon(fit); terminal.open(element); fit.fit();
      ready = true;
      const resize = new ResizeObserver(() => { if (terminal) { terminal.options.fontSize = element.clientWidth < 420 ? 11 : 13; fit.fit(); } });
      resize.observe(element);
      cleanup = () => { resize.disconnect(); terminal?.dispose(); };
    })();
    return () => { disposed = true; cleanup(); };
  });
</script>
<div class="demo-agent-terminal">
  <div class="demo-terminal" class:is-codex={agent === "codex"} bind:this={element}></div>
  {#if agent === "codex"}<HeroCodexChrome {input} {directory} />{/if}
</div>
