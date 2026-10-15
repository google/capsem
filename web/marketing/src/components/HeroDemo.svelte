<script lang="ts">
  import { onMount } from "svelte";
  import "@xterm/xterm/css/xterm.css";
  import { HERO_DEMO as demo, HERO_TERMINAL_SCRIPT } from "$lib/data";
  import { terminalPlayback } from "$lib/hero-terminal";

  let title = $state<string>(demo.shellTitle);
  let paused = $state(false);
  let preview = $state(false);
  let mounted = $state(false);
  let reducedMotion = $state(false);
  let terminalElement: HTMLDivElement;
  let container: HTMLDivElement;
  let restart = () => {};

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
      }, () => { paused = true; });
      restart = () => {
        player.reset(); title = demo.shellTitle; preview = false; paused = false;
        if (reducedMotion) { terminal.write(demo.summary); paused = true; }
      };
      const motion = matchMedia("(prefers-reduced-motion: reduce)");
      const updateMotion = () => {
        reducedMotion = motion.matches;
        terminal.options.cursorBlink = !motion.matches;
        if (motion.matches) { player.reset(); terminal.write(demo.summary); preview = false; paused = true; }
      };
      updateMotion();
      motion.addEventListener("change", updateMotion);
      let visible = false;
      const intersection = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; });
      intersection.observe(container);
      const resize = new ResizeObserver(() => {
        terminal.options.fontSize = container.clientWidth < 450 ? 11 : 13;
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
      <div class="demo-playback">
        {#if !reducedMotion}<button aria-label={paused ? demo.play : demo.pause} onclick={() => { paused = !paused; }}>{paused ? "▶" : "Ⅱ"}</button>{/if}
        <button aria-label={demo.replay} onclick={() => restart()}>↺</button>
      </div>
    </div>
    <div class="demo-terminal" bind:this={terminalElement}></div>
    {#if !mounted}<pre class="demo-fallback">$ {HERO_TERMINAL_SCRIPT[0].type}</pre>{/if}
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
  <p class="sr-only">{demo.descriptionForScreenReaders}</p>
</div>
