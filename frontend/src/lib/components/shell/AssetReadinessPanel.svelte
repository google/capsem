<script lang="ts">
  import Warning from 'phosphor-svelte/lib/Warning';
  import CircleNotch from 'phosphor-svelte/lib/CircleNotch';
  import type { AssetHealth } from '../../types/gateway';
  import { formatBytes } from '../../format';

  let {
    health = null,
    serviceReady = true,
    showActions = true,
    retrying = false,
    retryError = null,
    onretry,
    onrefresh,
  }: {
    health?: AssetHealth | null;
    serviceReady?: boolean;
    showActions?: boolean;
    retrying?: boolean;
    retryError?: string | null;
    onretry?: () => void | Promise<void>;
    onrefresh?: () => void | Promise<void>;
  } = $props();

  type PanelState = {
    title: string;
    message: string;
    details: string[];
    showRetry: boolean;
    severity: 'error' | 'warning';
  };

  let panelState = $derived.by<PanelState>(() => {
    if (!serviceReady) {
      return {
        title: 'Capsem service is offline',
        message: 'Start or recover the service before creating sessions.',
        details: [],
        showRetry: false,
        severity: 'error',
      };
    }

    if (!health) {
      return {
        title: 'VM asset status is unknown',
        message: 'Waiting for the service to report rootfs and manifest readiness.',
        details: [],
        showRetry: false,
        severity: 'warning',
      };
    }

    const details = (health.saved_vm_dependencies ?? []).map(dep =>
      `${dep.vm} missing ${dep.missing.join(', ')} (${dep.recovery_hint})`,
    );

    if (health.ready) {
      return {
        title: 'VM assets are ready',
        message: 'The selected profile assets are installed and verified.',
        details,
        showRetry: false,
        severity: 'warning',
      };
    }

    if (health.state === 'checking') {
      return {
        title: 'VM assets are being checked',
        message: 'Waiting for the service to verify rootfs and manifest readiness.',
        details,
        showRetry: false,
        severity: 'warning',
      };
    }

    if (health.state === 'updating') {
      const progress = health.progress
        ? `Updating ${health.progress.logical_name}.`
        : 'Required VM assets are updating.';
      return {
        title: 'VM assets are updating',
        message: progress,
        details,
        showRetry: false,
        severity: 'warning',
      };
    }

    if (health.state === 'error') {
      return {
        title: 'VM assets need attention',
        message: health.error ?? 'The service could not prepare required VM assets.',
        details,
        showRetry: health.retryable,
        severity: 'error',
      };
    }

    const missing = health.missing.length > 0
      ? `Missing: ${health.missing.join(', ')}.`
      : 'Required VM assets are not ready.';
    return {
      title: 'VM assets are missing',
      message: `${missing} The service will keep trying in the background.`,
      details,
      showRetry: false,
      severity: 'warning',
    };
  });

  let progressPercent = $derived.by(() => {
    const progress = health?.progress;
    if (!progress?.bytes_total || progress.bytes_total <= 0) return null;
    return Math.max(0, Math.min(100, Math.round((progress.bytes_done / progress.bytes_total) * 100)));
  });

  let profileLabel = $derived.by(() => {
    if (!health?.profile_id) return null;
    return health.profile_revision ? `${health.profile_id}@${health.profile_revision}` : health.profile_id;
  });
</script>

<div class="rounded-lg border p-4 text-sm {panelState.severity === 'error' ? 'border-destructive/30 bg-destructive/10' : 'border-warning/30 bg-warning/10'}">
  <div class="flex items-start gap-x-3">
    {#if health?.state === 'checking' || health?.state === 'updating'}
      <CircleNotch size={18} class="mt-0.5 shrink-0 text-warning animate-spin" />
    {:else}
      <Warning size={18} class="{panelState.severity === 'error' ? 'text-destructive' : 'text-warning'} mt-0.5 shrink-0" />
    {/if}

    <div class="min-w-0 flex-1">
      <div class="flex flex-wrap items-center gap-2">
        <p class="font-medium text-foreground">{panelState.title}</p>
        {#if profileLabel}
          <span class="rounded-full border border-line-2 bg-layer px-2 py-0.5 font-mono text-[11px] text-foreground">{profileLabel}</span>
        {/if}
      </div>
      <p class="mt-0.5 text-muted-foreground-1">{panelState.message}</p>

      {#if health?.progress}
        <div class="mt-3">
          <div
            role="progressbar"
            aria-label="Profile asset download progress"
            aria-valuemin="0"
            aria-valuemax={progressPercent == null ? undefined : 100}
            aria-valuenow={progressPercent == null ? undefined : progressPercent}
            class="h-2 w-full overflow-hidden rounded-full bg-layer border border-layer-line"
          >
            <div
              class="h-full rounded-full bg-primary transition-all"
              style={progressPercent == null ? 'width: 33%' : `width: ${progressPercent}%`}
            ></div>
          </div>
          <div class="mt-1 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground-1">
            <span class="font-mono">{health.progress.logical_name}</span>
            <span>
              {formatBytes(health.progress.bytes_done)}
              {#if health.progress.bytes_total != null}
                / {formatBytes(health.progress.bytes_total)}
              {/if}
              {#if progressPercent != null}
                ({progressPercent}%)
              {/if}
            </span>
          </div>
        </div>
      {/if}

      {#if health && health.missing.length > 0}
        <p class="mt-2 text-xs text-muted-foreground">Missing: {health.missing.join(', ')}</p>
      {/if}

      {#if panelState.details.length > 0}
        <ul class="mt-2 space-y-1">
          {#each panelState.details as detail}
            <li class="text-xs text-muted-foreground">{detail}</li>
          {/each}
        </ul>
      {/if}

      {#if showActions}
        <div class="mt-3 flex flex-wrap items-center gap-2">
          {#if panelState.showRetry && onretry}
            <button
              type="button"
              class="py-1 px-3 text-xs font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              disabled={retrying}
              onclick={onretry}
            >
              {retrying ? 'Retrying...' : 'Retry setup'}
            </button>
          {/if}
          {#if onrefresh}
            <button
              type="button"
              class="py-1 px-3 text-xs font-medium rounded-lg bg-layer border border-layer-line text-layer-foreground hover:bg-layer-hover transition-colors"
              onclick={onrefresh}
            >
              Refresh status
            </button>
          {/if}
        </div>
      {/if}

      {#if retryError}
        <p class="mt-2 text-xs text-destructive">{retryError}</p>
      {/if}
    </div>
  </div>
</div>
