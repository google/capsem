<script lang="ts">
  import { onMount } from "svelte";
  import { DEMO } from "$lib/data";
  // A presentation script, not a live VM or a performance measurement.
  let count = $state<number>(DEMO.lines.length);
  let running = $state(false);
  let reducedMotion = $state(false);
  const complete = $derived(count >= DEMO.lines.length);
  onMount(() => {
    const preference = window.matchMedia("(prefers-reduced-motion: reduce)");
    reducedMotion = preference.matches;
    const update = () => {
      reducedMotion = preference.matches;
      if (reducedMotion) { running = false; count = DEMO.lines.length; }
    };
    preference.addEventListener("change", update);
    // Play once; never loop or animate for reduced-motion users.
    if (!reducedMotion) { count = 0; running = true; }
    return () => preference.removeEventListener("change", update);
  });
  $effect(() => {
    if (!running || complete || reducedMotion) return;
    const timer = setTimeout(() => { count += 1; if (count === DEMO.lines.length) running = false; }, count < 4 ? 600 : 950);
    return () => clearTimeout(timer);
  });
  function toggle() {
    if (complete) count = 0;
    running = !running;
  }
</script>
<div class="demo-scene">
  <div class="orbit orbit-one" aria-hidden="true"></div><div class="orbit orbit-two" aria-hidden="true"></div>
  <div class="workspace-window">
    <div class="window-toolbar"><div class="window-dots" aria-hidden="true"><i></i><i></i><i></i></div><span>{DEMO.title}</span><span class="window-icon" aria-hidden="true">⌘</span></div>
    <div class="workspace-header"><span class="session-icon" aria-hidden="true">▧</span><strong>{DEMO.session}</strong><span class="status-badge"><i></i>{DEMO.badge}</span></div>
    <div class="terminal" role="img" aria-label={DEMO.lines.map(line => line.text).join(". ")}>
      {#each DEMO.lines as line, i}<div class="terminal-line {line.kind}" class:visible={i < count} aria-hidden="true">{line.text}</div>{/each}
      <span class="terminal-cursor" class:visible={running} aria-hidden="true"></span>
    </div>
    <div class="workspace-footer"><span><i></i>{DEMO.connection}</span><span>{DEMO.boundary}</span></div>
  </div>
  <div class="demo-caption"><span>{DEMO.label}</span><div class="demo-steps" aria-hidden="true">{#each DEMO.steps as step, i}<span class:active={i === (count < 2 ? 0 : count < 5 ? 1 : 2)}>{step}</span>{/each}</div>
    {#if !reducedMotion}<button onclick={toggle}>{running ? DEMO.pause : complete ? DEMO.replay : DEMO.play}<span aria-hidden="true">{running ? "Ⅱ" : "↻"}</span></button>{/if}
  </div>
</div>
