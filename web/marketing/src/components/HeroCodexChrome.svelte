<script lang="ts">
  import { HERO_DEMO } from "$lib/data";
  let { input = null, directory = HERO_DEMO.codex.directory }: { input?: string | null; directory?: string } = $props();
  const copy = HERO_DEMO.codex;
</script>
<div class="demo-codex-header" aria-hidden="true">
  <strong><span>&gt;_</span> {copy.name}</strong>
  <dl><dt>{copy.modelLabel}</dt><dd>{copy.model} <span>{copy.changeModel}</span></dd><dt>{copy.directoryLabel}</dt><dd>{directory}</dd></dl>
</div>
<div class="demo-codex-composer" aria-hidden="true">
  <div><span>›</span><span class:demo-input-active={input !== null}>{input ?? copy.placeholder}{#if input !== null}<i aria-hidden="true"></i>{/if}</span></div>
  <footer>{copy.context}</footer>
</div>
