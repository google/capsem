<script lang="ts">
  interface Props {
    text: string;
  }

  let { text }: Props = $props();
</script>

<span class="inline-block rounded-full border border-border bg-badge-bg px-3 py-1 text-xs font-medium text-badge-text">
  {text}
</span>
