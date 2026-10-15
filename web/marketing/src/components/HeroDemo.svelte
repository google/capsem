<script lang="ts">
  import { onMount } from "svelte";
  import "@xterm/xterm/css/xterm.css";
  import { HERO_DEMO as demo, HERO_TERMINAL_SCRIPT } from "$lib/data";
  import { terminalPlayback, stepDuration } from "$lib/hero-terminal";

  let duration = 0;
  const chapters = HERO_TERMINAL_SCRIPT.flatMap((step, index) => {
    const start = duration;
    duration += stepDuration(step);
    return step.chapter === undefined ? [] : [{ index, start, label: demo.chapters[step.chapter] }];
  });

  let title = $state<string>(demo.shellTitle);
  let paused = $state(false);
  let preview = $state(false);
  let mounted = $state(false);
  let reducedMotion = $state(false);
  let insert = $state<"build" | "work" | null>(null);
  let codex = $state(false);
  let position = $state(0);
  let activeChapter = $state(0);
  let finished = $state(false);
  let terminalElement: HTMLDivElement;
  let container: HTMLDivElement;
  let restart = () => {};
  let seek = (_index: number) => {};
  const progress = (index: number) => Math.max(0, Math.min(1,
    (position - chapters[index].start) / ((chapters[index + 1]?.start ?? duration) - chapters[index].start)));
  function togglePlayback() {
    if (finished) restart();
    else paused = !paused;
  }

  onMount(() => {
    let disposed = false;
    let cleanup = () => {};
    async function initialize() {
      const [{ Terminal }, { FitAddon }] = await Promise.all([
        import("@xterm/xterm"), import("@xterm/addon-fit"),
      ]);
      if (disposed) return;
      const style = getComputedStyle(container);
      const terminal = new Terminal({
        disableStdin: true, cursorBlink: true, cursorStyle: "block",
        fontFamily: '"SFMono-Regular", Menlo, Consolas, monospace',
        fontSize: container.clientWidth < 450 ? 11 : 13,
        lineHeight: 1.35, scrollback: 100, convertEol: true,
        theme: {
          background: style.getPropertyValue("--color-demo-shell").trim(),
          foreground: style.getPropertyValue("--color-demo-text").trim(),
          cursor: style.getPropertyValue("--color-demo-text").trim(),
          selectionBackground: style.getPropertyValue("--color-demo-selection").trim(),
        },
      });
      const fit = new FitAddon();
      terminal.loadAddon(fit);
      terminal.open(terminalElement);
      fit.fit();
      mounted = true;
      const player = terminalPlayback(terminal, HERO_TERMINAL_SCRIPT, (step) => {
        if (step.title) title = step.title;
        if (step.preview !== undefined) preview = step.preview;
        if (step.insert !== undefined) insert = step.insert;
        if (step.codex !== undefined) codex = step.codex;
        if (step.chapter !== undefined) activeChapter = step.chapter;
      }, () => { paused = true; finished = true; }, (milliseconds) => { position = milliseconds; });
      const resetView = () => {
        title = demo.shellTitle; preview = false; insert = null; codex = false;
        activeChapter = 0; finished = false;
      };
      restart = () => {
        resetView(); player.reset(); paused = false;
        if (reducedMotion) showSummary();
      };
      seek = (index) => {
        resetView(); player.seek(chapters[index].index); paused = false;
      };
      const showSummary = () => {
        resetView(); player.reset(); terminal.write(demo.summary);
        title = demo.codexTitle; codex = true; position = duration;
        activeChapter = chapters.length - 1; paused = true; finished = true;
      };
      const motion = matchMedia("(prefers-reduced-motion: reduce)");
      const updateMotion = () => {
        reducedMotion = motion.matches;
        terminal.options.cursorBlink = !motion.matches;
        if (motion.matches) showSummary();
      };
      updateMotion();
      motion.addEventListener("change", updateMotion);
      let visible = false;
      const intersection = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; });
      intersection.observe(container);
      const resize = new ResizeObserver(() => {
        terminal.options.fontSize = container.clientWidth < 450 ? 11 : codex ? 12 : 13;
        fit.fit();
      });
      resize.observe(terminalElement);
      let previous = performance.now();
      const timer = setInterval(() => {
        const now = performance.now();
        const delta = Math.min(now - previous, 100);
        previous = now;
        if (!paused && !reducedMotion && visible && !document.hidden) player.tick(delta);
      }, 30);
      cleanup = () => {
        clearInterval(timer); motion.removeEventListener("change", updateMotion);
        intersection.disconnect(); resize.disconnect(); terminal.dispose();
      };
    }
    void initialize();
    return () => { disposed = true; cleanup(); };
  });
</script>

<div class="hero-demo" bind:this={container} role="group" aria-label={demo.terminalLabel}>
  <div class="demo-window">
    <div class="demo-titlebar">
      <span class="demo-traffic-lights" aria-hidden="true"><i></i><i></i><i></i></span>
      <span class="demo-window-title">{title}</span>
    </div>
    <div class="demo-terminal" class:is-codex={codex} bind:this={terminalElement}></div>
    {#if !mounted}<pre class="demo-fallback">$ {HERO_TERMINAL_SCRIPT[0].type}</pre>{/if}
    {#if codex}
      <div class="demo-codex-composer" aria-hidden="true">
        <div><span>›</span><span>{demo.codex.placeholder}</span></div>
        <footer><span>{demo.codex.status}</span><span>{demo.codex.shortcuts}</span></footer>
      </div>
    {/if}
    {#if insert}
      {@const message = demo.inserts[insert]}
      <div class="demo-insert" class:demo-insert-work={insert === "work"}>
        <span>{message.eyebrow}</span>
        <h3>{message.title}</h3>
        <p>{message.detail}</p>
        <a href={message.href}>{message.link}</a>
      </div>
    {/if}
  </div>
  {#if preview}
    <div class="demo-browser" role="group" aria-label={demo.previewLabel}>
      <div class="demo-address"><span aria-hidden="true">‹ ›</span><code>{demo.previewURL}</code><button aria-label={demo.closePreview} onclick={() => { preview = false; }}>×</button></div>
          <div class="demo-website">
            <header><strong>{demo.preview.brand}</strong><span>{demo.preview.nav}</span></header>
            <div class="demo-site-content">
              <div><small>{demo.preview.eyebrow}</small><h3>{demo.preview.title}</h3><p>{demo.preview.description}</p><span class="demo-site-cta">{demo.preview.action}</span></div>
              <div class="demo-sculpture" aria-hidden="true"><div></div><span>{demo.preview.monogram}</span></div>
            </div>
            <footer>{demo.preview.footer}<span>© 2026</span></footer>
          </div>

    </div>
  {/if}
  <div class="demo-player">
    <nav class="demo-chapters" aria-label={demo.chaptersLabel}>
      {#each chapters as chapter, index}
        <button class:active={activeChapter === index} aria-current={activeChapter === index ? "step" : undefined} disabled={reducedMotion || !mounted} onclick={() => seek(index)}>
          <span class="demo-progress" aria-hidden="true"><span style:width={`${progress(index) * 100}%`}></span></span>
          <span>{chapter.label}</span>
        </button>
      {/each}
    </nav>
    <div class="demo-playback">
      {#if !reducedMotion}<button aria-label={paused ? demo.play : demo.pause} disabled={!mounted} onclick={togglePlayback}>{paused ? "▶" : "Ⅱ"}</button>{/if}
      <button aria-label={demo.replay} disabled={!mounted} onclick={() => restart()}>↺</button>
    </div>
  </div>
  <p class="sr-only">{demo.descriptionForScreenReaders}</p>
</div>
