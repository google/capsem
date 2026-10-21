<script lang="ts">
  import { onMount } from "svelte";
  import "@xterm/xterm/css/xterm.css";
  import { HERO_DEMO as demo, HERO_TERMINAL_SCRIPT } from "$lib/data";
  import { terminalPlayback, stepDuration } from "$lib/hero-terminal";
  import type { TerminalStep } from "$lib/hero-terminal";
  import HeroScale from "./HeroScale.svelte";
  import HeroTui from "./HeroTui.svelte";

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
  let insert = $state<TerminalStep["insert"]>(null);
  let codex = $state(false);
  let scale = $state(-1);
  let surface = $state<"gui" | "tui" | "api" | "mcp">("gui");
  let showGui = $derived(scale >= 0 && surface === "gui");
  let desktopApp: "app" | "browser" | "terminal" = $derived(showGui ? "app" : preview ? "browser" : "terminal");
  let windowOpen = $state(false);
  let cursor = $state<NonNullable<TerminalStep["cursor"]>>("start");
  let dock = $state<TerminalStep["dock"]>(null);
  let clicking = $state(false);
  let mouseX = $state(82);
  let mouseY = $state(68);
  let dockFocus = $derived(!insert && mouseY > 89 && (cursor === "terminal" || cursor === "app") ? cursor : dock);
  let tui = $state(false);
  let overlay = $state<TerminalStep["overlay"]>(null);
  let workflow = $state<"website" | "email">("website");
  let email = $state(false);
  let draft = $state("");
  let profile = $state<"code" | "co-work">("code");
  let key = $state<string | null>(null);
  let guiTerminal = $state(false);
  let input = $state<string | null>(null);
  let position = $state(0);
  let activeChapter = $state(0);
  let finished = $state(false);
  let terminalElement: HTMLDivElement;
  let container: HTMLDivElement;
  let desktop: HTMLDivElement;
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
        if (step.window !== undefined) windowOpen = step.window;
        if (step.cursor !== undefined) cursor = step.cursor;
        if (step.dock !== undefined) dock = step.dock;
        if (step.click !== undefined) clicking = step.click;
        if (step.tui !== undefined) {
          tui = step.tui;
          terminal.options.theme = { ...terminal.options.theme, background: style.getPropertyValue(tui ? "--color-demo-tui" : "--color-demo-shell").trim() };
        }
        if (step.overlay !== undefined) overlay = step.overlay;
        if (step.overlay === "create") draft = step.type ?? "";
        if (step.workflow !== undefined) workflow = step.workflow;
        if (step.email !== undefined) email = step.email;
        if (step.profile !== undefined) profile = step.profile;
        if (step.key !== undefined) key = step.key;
        if (step.guiTerminal !== undefined) guiTerminal = step.guiTerminal;
      }, () => { paused = true; finished = true; }, (milliseconds) => { position = milliseconds; }, (text) => {
        if (overlay === "create" && text !== null) draft = text;
        else input = text;
      });
      const resetView = () => {
        title = demo.shellTitle; preview = false; insert = null; codex = false; input = null; scale = -1; surface = "gui";
        windowOpen = false; cursor = "start"; dock = null; clicking = false; tui = false; overlay = null;
        workflow = "website"; email = false; draft = ""; profile = "code"; key = null; guiTerminal = false;
        mouseX = 82; mouseY = 68;
        terminal.options.theme = { ...terminal.options.theme, background: style.getPropertyValue("--color-demo-shell").trim() };
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
        windowOpen = true;
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
        if (!paused && !reducedMotion && visible && !document.hidden) {
          player.tick(delta);
          let target = { x: 78, y: 55 };
          const bounds = desktop.getBoundingClientRect();
          const point = (x: number, y: number) => ({ x: (x - bounds.left) / bounds.width * 100, y: (y - bounds.top) / bounds.height * 100 });
          if (cursor === "terminal" || cursor === "app" || cursor === "session") {
            const selector = cursor === "session" ? ".demo-sessions tbody tr td" : cursor === "terminal" ? ".demo-dock > span:first-child img" : ".demo-dock > span:last-child img";
            const rect = desktop.querySelector(selector)?.getBoundingClientRect();
            if (rect) target = point(rect.left + rect.width / 2, rect.top + rect.height / 2);
          } else if (cursor === "link") {
            const screen = terminalElement.querySelector(".xterm-screen")?.getBoundingClientRect();
            if (screen) for (let row = 0; row < terminal.rows; row++) {
              const text = terminal.buffer.active.getLine(terminal.buffer.active.viewportY + row)?.translateToString() ?? "";
              const column = text.indexOf("http://localhost:3000");
              if (column >= 0) target = point(screen.left + (column + 10) / terminal.cols * screen.width, screen.top + (row + .5) / terminal.rows * screen.height);
            }
          } else if (cursor === "preview") target = { x: 79, y: 37 };
          else if (cursor === "start") target = { x: 82, y: 68 };
          const blend = Math.min(1, delta / 120);
          mouseX += (target.x - mouseX) * blend;
          mouseY += (target.y - mouseY) * blend;
        }
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

<div class="hero-demo" class:demo-paused={paused} bind:this={container} role="group" aria-label={demo.terminalLabel}>
  <div class="demo-desktop" bind:this={desktop}>
    <div class="demo-menubar" aria-hidden="true">
      <img src="/logo.svg" alt="" /><strong>{demo.desktop[desktopApp]}</strong>
      {#each demo.desktop.menus as menu}<span>{menu}</span>{/each}
      <time>{demo.desktop.clock}</time>
    </div>
  <div class="demo-window" class:demo-window-open={windowOpen} class:demo-window-behind={showGui} class:demo-window-tui={tui} aria-hidden={!windowOpen || showGui} inert={!windowOpen || showGui}>
    <div class="demo-titlebar">
      <span class="demo-traffic-lights" aria-hidden="true"><i></i><i></i><i></i></span>
      <span class="demo-window-title">{title}</span>
    </div>
    <div class="demo-terminal" class:is-codex={codex} bind:this={terminalElement}></div>
    {#if !mounted}<pre class="demo-fallback">$ {HERO_TERMINAL_SCRIPT.find(step => step.type)?.type}</pre>{/if}
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
    {#if tui}<HeroTui overlay={overlay ?? null} {workflow} {email} parallel={scale} {draft} {profile} />{/if}
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
    {#if showGui}<HeroScale active={scale} terminal={guiTerminal} />{/if}
    <div class="demo-dock" data-focus={dockFocus ?? ""} class:demo-dock-click={clicking} aria-hidden="true">
      <span class:current={desktopApp === "terminal"}><img src="/demo/terminal.png" width="40" height="40" alt="" /></span>
      <span class:current={desktopApp === "browser"}><img src="/demo/chrome.png" width="40" height="40" alt="" /></span>
      <span class:current={desktopApp === "app"}><img src="/demo/capsem.png" width="40" height="40" alt="" /></span>
    </div>
    {#if key}<kbd class="demo-keystroke" aria-hidden="true">{key}</kbd>{/if}
    {#if !reducedMotion && !insert}
      <div class="demo-pointer" class:clicking style:left={`${mouseX}%`} style:top={`${mouseY}%`} aria-hidden="true"><svg viewBox="0 0 24 30"><path d="M3 2v24l6-6 5 9 4-2-5-9h9Z" /></svg>{#if clicking}<i></i>{/if}</div>
    {/if}
    {#if insert}
      {@const message = demo.inserts[insert]}
      <div class="demo-insert" class:demo-insert-work={insert === "work"}>
        <span>{message.eyebrow}</span><h3>{message.title}</h3><p>{message.detail}</p>
      </div>
    {/if}
  </div>
  <div class="demo-surfaces" class:visible={scale >= 0} role="group" aria-label={demo.scale.controlsLabel} aria-hidden={scale < 0}>
    {#each demo.scale.surfaces as mode}<span class:current={surface === mode.id}>{mode.label}</span>{/each}
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
