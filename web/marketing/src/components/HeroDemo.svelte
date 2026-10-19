<script lang="ts">
  import { onMount } from "svelte";
  import "@xterm/xterm/css/xterm.css";
  import { HERO_DEMO as demo, HERO_TERMINAL_SCRIPT } from "$lib/data";
  import { terminalPlayback, stepDuration } from "$lib/hero-terminal";
  import HeroScale from "./HeroScale.svelte";

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
  let scale = $state(-1);
  let surface = $state<"gui" | "tui" | "api" | "mcp">("gui");
  let desktopApp: "app" | "browser" | "terminal" = $derived(scale >= 0 && surface === "gui" ? "app" : preview ? "browser" : "terminal");
  let input = $state<string | null>(null);
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
        if (step.scale !== undefined) scale = step.scale;
        if (step.surface !== undefined) surface = step.surface;
      }, () => { paused = true; finished = true; }, (milliseconds) => { position = milliseconds; }, (text) => { input = text; });
      const resetView = () => {
        title = demo.shellTitle; preview = false; insert = null; codex = false; input = null; scale = -1; surface = "gui";
        activeChapter = 0; finished = false;
      };
      restart = () => {
        resetView(); player.reset(); paused = false;
        if (reducedMotion) showFinale();
      };
      seek = (index) => {
        resetView(); player.seek(chapters[index].index); paused = false;
      };
      const showFinale = () => {
        resetView(); player.reset(); scale = demo.scale.sessions.length; position = duration;
        activeChapter = chapters.length - 1; paused = true; finished = true;
      };
      const motion = matchMedia("(prefers-reduced-motion: reduce)");
      const updateMotion = () => {
        reducedMotion = motion.matches;
        terminal.options.cursorBlink = !motion.matches;
        if (motion.matches) showFinale();
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
  <div class="demo-desktop">
    <div class="demo-menubar" aria-hidden="true">
      <img src="/logo.svg" alt="" /><strong>{demo.desktop[desktopApp]}</strong>
      {#each demo.desktop.menus as menu}<span>{menu}</span>{/each}
      <time>{demo.desktop.clock}</time>
    </div>
  <div class="demo-window" class:demo-window-behind={scale >= 0} aria-hidden={scale >= 0}>
    <div class="demo-titlebar">
      <span class="demo-traffic-lights" aria-hidden="true"><i></i><i></i><i></i></span>
      <span class="demo-window-title">{title}</span>
    </div>
    <div class="demo-terminal" class:is-codex={codex} bind:this={terminalElement}></div>
    {#if !mounted}<pre class="demo-fallback">$ {HERO_TERMINAL_SCRIPT[0].type}</pre>{/if}
    {#if codex}
      <div class="demo-codex-header" aria-hidden="true">
        <strong><span>&gt;_</span> {demo.codex.name}</strong>
        <dl>
          <dt>{demo.codex.modelLabel}</dt><dd>{demo.codex.model} <span>{demo.codex.changeModel}</span></dd>
          <dt>{demo.codex.directoryLabel}</dt><dd>{demo.codex.directory}</dd>
        </dl>
      </div>
      <div class="demo-codex-composer" aria-hidden="true">
        <div><span>›</span><span class:demo-input-active={input !== null}>{input ?? demo.codex.placeholder}{#if input !== null}<i class:paused aria-hidden="true"></i>{/if}</span></div>
        <footer>{demo.codex.context}</footer>
      </div>
    {/if}
    {#if insert}
      {@const message = demo.inserts[insert]}
      <div class="demo-insert" class:demo-insert-work={insert === "work"}>
        <span>{message.eyebrow}</span>
        <h3>{message.title}</h3>
        <p>{message.detail}</p>
      </div>
    {/if}
  </div>
  {#if preview}
    <div class="demo-browser" role="group" aria-label={demo.previewLabel}>
      <div class="demo-address"><span aria-hidden="true">‹ ›</span><code>{demo.previewURL}</code><span aria-hidden="true">×</span></div>
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
    {#if scale >= 0}<HeroScale active={scale} {surface} />{/if}
    <div class="demo-dock" aria-hidden="true">
      <span class:current={desktopApp === "terminal"}><span class="demo-dock-terminal">&gt;_</span></span>
      <span class:current={desktopApp === "browser"}><span class="demo-dock-browser">◈</span></span>
      <span class:current={desktopApp === "app"}><span class="demo-dock-app"><img src="/logo.svg" alt="" /></span></span>
    </div>
  </div>
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
