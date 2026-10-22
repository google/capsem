<script lang="ts">
  import { getBaseUrl } from '../../api';
  import { isSurfaceReconnectMessage, surfaceEmbedUrl } from '../../models/surface';
  import { vmStore } from '../../stores/vms.svelte';
  import { metricMs, metricSize, parseSurfaceMetrics, type SurfaceMetrics } from '../../models/surface-metrics';

  let { vmId, revision = 0, debug = false }: { vmId: string; revision?: number; debug?: boolean } = $props();
  let frame: HTMLIFrameElement | undefined = $state();
  let metrics: SurfaceMetrics | null = $state(null);
  let reconnect = $state(0);
  let src = $derived(surfaceEmbedUrl(getBaseUrl(), vmId, window.location.origin));
  // Keep the graphical connection across UI/Terminal/Stats switches. A VM
  // lifecycle transition starts a fresh bootstrap against the current owner.
  let status = $derived(vmStore.vms.find(vm => vm.id === vmId)?.status);

  function sendDebug() {
    // No credential or authority is carried in this flag. The child accepts
    // it only from its bootstrap's exact trusted parent origin.
    frame?.contentWindow?.postMessage({type: 'capsem:surface-debug', enabled: debug}, '*');
  }
  $effect(() => {
    void debug;
    metrics = null;
    sendDebug();
  });
</script>

<svelte:window onmessage={event => {
  if (isSurfaceReconnectMessage(event, frame?.contentWindow)) reconnect += 1;
  if (event.source !== frame?.contentWindow) return;
  if (event.data?.type === 'capsem:surface-debug-ready') sendDebug();
  if (debug && event.data?.type === 'capsem:surface-metrics') metrics = parseSurfaceMetrics(event.data.metrics);
}} />

{#key `${status}:${revision}:${reconnect}`}
  <iframe
    bind:this={frame}
    {src}
    title="UI: {vmId}"
    sandbox="allow-scripts allow-same-origin allow-forms"
    referrerpolicy="no-referrer"
    class="w-full h-full border-0"
    onload={() => { metrics = null; sendDebug(); }}
  ></iframe>
{/key}

{#if debug}
  <aside aria-label="Display diagnostics" class="pointer-events-none absolute right-3 top-3 z-20 rounded-lg border border-line-2 bg-background/95 p-3 font-mono text-xs text-foreground shadow-lg">
    <div class="mb-1 flex items-baseline gap-3">
      <strong class="text-xl text-primary">{metrics?.fps == null ? '—' : metrics.fps.toFixed(0)} FPS</strong>
      <span class="text-muted-foreground-1">display updates</span>
    </div>
    {#if metrics}
      <div>Frame {metricMs(metrics.frameMs)} · Paint {metricMs(metrics.paintMs)}</div>
      <div>Decode + queue {metricMs(metrics.decodeMs)} · RTT {metricMs(metrics.rttMs)}</div>
      <div title="Time from the first outstanding input to the next presented update; not proof of a causally related guest response.">Mouse → next paint {metricMs(metrics.mouseMs)} (proxy)</div>
      <div>Key → next paint {metricMs(metrics.keyMs)} (proxy)</div>
      <div class="mt-2">Codec {metrics.codec ?? '—'} · Q {metrics.quality ?? '—'}</div>
      <div>Wire color {metrics.color ?? 'not reported'} · Depth {metrics.depth == null ? 'not reported' : `${metrics.depth} bit`}</div>
      <div>Source {metricSize(metrics.sourceWidth, metrics.sourceHeight)}</div>
      <div>Canvas {metricSize(metrics.backingWidth, metrics.backingHeight)}</div>
      <div>Display {metricSize(metrics.displayWidth, metrics.displayHeight)} · DPR {metrics.dpr ?? '—'}</div>
      <div class="text-muted-foreground-1">Decode errors {metrics.decodeErrors ?? 0} · Idle = 0 FPS</div>
    {:else}
      <span class="text-muted-foreground-1">Waiting for viewer metrics…</span>
    {/if}
  </aside>
{/if}
