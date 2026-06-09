<script lang="ts">
  let {
    label,
    value,
    tone = 'default',
  }: {
    label: string;
    value: string;
    tone?: 'default' | 'primary' | 'danger';
  } = $props();
</script>

<div class="bg-card border border-card-line rounded-lg p-3">
  <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">{label}</div>
  <div class="text-lg font-semibold {tone === 'primary' ? 'text-primary' : tone === 'danger' ? 'text-destructive' : 'text-foreground'}">{value}</div>
</div>
