<script lang="ts">
  import { onMount } from "svelte";
  import { HERO_DEMO as demo } from "$lib/data";

  let chapter = $state(0);
  let elapsed = $state(0);
  let paused = $state(false);
  let reducedMotion = $state(false);
  let visible = $state(false);
  let container: HTMLDivElement;
  const scene = $derived(demo.chapters[chapter]);
  const command = $derived(scene.command.slice(0, reducedMotion ? undefined : Math.floor(elapsed / 24)));
  const typingEnd = $derived(scene.command.length * 24 + 200);
  const shownLines = $derived(reducedMotion ? scene.lines.length : Math.max(0, Math.floor((elapsed - typingEnd) / 650)));
  const complete = $derived(reducedMotion || elapsed >= scene.duration - 100);

  function select(index: number) {
    chapter = index;
    elapsed = demo.chapters[index].duration - 50;
    paused = true;
  }

  function replay() {
    chapter = 0;
    elapsed = 0;
    paused = false;
  }

  onMount(() => {
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    const updateMotion = () => { reducedMotion = motion.matches; if (motion.matches) paused = true; };
    updateMotion();
    motion.addEventListener("change", updateMotion);
    const observer = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; }, { threshold: 0.15 });
    observer.observe(container);
    let previous = performance.now();
    const timer = window.setInterval(() => {
      const now = performance.now();
      const delta = Math.min(now - previous, 100);
      previous = now;
      if (paused || !visible || document.hidden || reducedMotion) return;
      elapsed += delta;
      if (elapsed >= scene.duration) {
        if (chapter === demo.chapters.length - 1) { paused = true; elapsed = scene.duration; }
        else { chapter += 1; elapsed = 0; }
      }
    }, 50);
    return () => { clearInterval(timer); observer.disconnect(); motion.removeEventListener("change", updateMotion); };
  });
</script>

<div class="hero-demo" bind:this={container} role="group" aria-label={demo.terminalLabel}>
  <div class="demo-window">
    <div class="demo-titlebar">
      <span class="demo-window-mark" aria-hidden="true">⌘</span>
      <span class="demo-context">{scene.context === "host" ? demo.host : scene.context === "inbox" ? demo.inboxVM : demo.websiteVM}</span>
      <span class="demo-boundary"><span></span>{scene.context === "host" ? demo.host : demo.isolated}</span>
    </div>
    <div class="demo-imagebar">
      <span>{demo.imageLabel}</span>
      <code>{scene.context === "host" ? "—" : scene.context === "inbox" ? "example/codex-mail:latest" : "example/agy:latest"}</code>
    </div>
    <div class="demo-stage">
      {#if scene.context === "preview"}
        <div class="demo-browser">
          <div class="demo-address"><span>{demo.previewLabel}</span><code>{demo.previewURL}</code><span aria-hidden="true">↗</span></div>
          <div class="demo-website">
            <header><strong>{demo.preview.brand}</strong><span>{demo.preview.nav}</span></header>
            <div class="demo-site-content">
              <div><small>{demo.preview.eyebrow}</small><h3>{demo.preview.title}</h3><p>{demo.preview.description}</p><span class="demo-site-cta">{demo.preview.action}</span></div>
              <div class="demo-sculpture" aria-hidden="true"><div></div><span>{demo.preview.monogram}</span></div>
            </div>
            <footer>{demo.preview.footer}<span>© 2026</span></footer>
          </div>
        </div>
      {:else}
        <div class="demo-terminal" class:demo-agy={chapter === 2} class:demo-mail={scene.context === "inbox"}>
          {#if chapter === 2}
            <div class="demo-agy-brand">
              <svg viewBox="0 0 13 12" width="66" height="61" aria-hidden="true">
                <defs><linearGradient id="agy-spectrum" x1="0" y1="0" x2="0.6" y2="1"><stop offset="0%" stop-color="var(--color-agy-orange)"/><stop offset="35%" stop-color="var(--color-agy-yellow)"/><stop offset="60%" stop-color="var(--color-agy-purple)"/><stop offset="100%" stop-color="var(--color-agy-blue)"/></linearGradient></defs>
                <path fill="url(#agy-spectrum)" d="M6 0h1v1h1v1h1v2h1v2h1v2h1v2h1v1h-3V9H9V7H8V5H7V4H6v1H5v2H4v2H3v2H0v-1h1V8h1V6h1V4h1V2h1V1h1Z"/>
              </svg>
              <div><strong>{demo.agy.name}</strong><span>{demo.agy.model}</span><span>{demo.agy.workspace}</span></div>
            </div>
          {/if}
          <div class="demo-command"><span class="demo-prompt">{scene.context === "website" && chapter === 2 ? demo.agy.prompt : "$"}</span><code>{command}<span class:blinking={!paused && !reducedMotion} class="demo-cursor" aria-hidden="true">▍</span></code></div>
          <div class="demo-output">
            {#each scene.lines.slice(0, shownLines) as line}<div class:demo-success={line.startsWith("✓")}>{line}</div>{/each}
          </div>
          {#if scene.context === "inbox" && (shownLines >= scene.lines.length || complete)}
            <div class="demo-inbox" aria-label={demo.inboxLabel}>
              <div class="demo-inbox-heading">{demo.inboxLabel}</div>
              {#each demo.emails as email}<div class="demo-email"><span class="demo-avatar">{email.sender.charAt(0)}</span><div><strong>{email.sender}</strong><span>{email.subject}</span></div><time>{email.time}</time></div>{/each}
            </div>
          {/if}
        </div>
      {/if}
    </div>
    <div class="demo-story" aria-live="off">
      <span class="demo-kicker">{scene.kicker}</span>
      <h3>{scene.title}</h3>
      <p>{scene.detail}</p>
      <a href={scene.href}>{scene.link}</a>
    </div>
  </div>
  <div class="demo-controls">
    <div class="demo-chapters" aria-label="Demo chapters">
      {#each demo.chapters as item, index}
        <button class:active={chapter === index} aria-current={chapter === index ? "step" : undefined} onclick={() => select(index)}>
          <span class="demo-chapter-track"><span style:width={index < chapter ? "100%" : index === chapter ? `${reducedMotion ? 100 : elapsed / scene.duration * 100}%` : "0%"}></span></span>
          <span>{item.label}</span>
        </button>
      {/each}
    </div>
    <div class="demo-playback">
      {#if !reducedMotion}<button onclick={() => { paused = !paused; }} aria-label={paused ? demo.play : demo.pause}>{paused ? "▶" : "Ⅱ"}</button>{/if}
      <button onclick={replay} aria-label={demo.replay}>↺</button>
    </div>
  </div>
  <p class="demo-disclaimer">{demo.demoLabel}</p>
</div>
