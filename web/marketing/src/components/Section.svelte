<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    id?: string;
    dark?: boolean;
    alt?: boolean;
    narrow?: boolean;
    class?: string;
    children: Snippet;
  }

  let { id, dark = false, alt = false, narrow = false, class: cls = "", children }: Props = $props();
</script>

<section
  {id}
  class="{dark ? 'section-dark' : alt ? 'bg-surface-alt' : 'bg-surface'} py-24 md:py-32 {cls}"
>
  <div class="mx-auto {narrow ? 'max-w-4xl' : 'max-w-6xl'} px-6">
    {@render children()}
  </div>
</section>
