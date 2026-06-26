<script lang="ts">
  let {
    value,
    kind = 'tag',
  }: {
    value: string;
    kind?: 'tag' | 'decision' | 'detection';
  } = $props();

  let css = $derived.by(() => {
    if (kind === 'decision') {
      return value === 'allowed' ? 'bg-primary/10 text-primary' : 'bg-destructive/10 text-destructive';
    }
    if (kind === 'detection') {
      return value === 'critical' || value === 'high'
        ? 'bg-destructive/10 text-destructive'
        : value === 'none'
          ? 'bg-muted text-muted-foreground-1'
          : 'bg-warning/15 text-warning-foreground';
    }
    return 'bg-muted text-muted-foreground-1';
  });
</script>

<span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium {css}">{value || 'none'}</span>
