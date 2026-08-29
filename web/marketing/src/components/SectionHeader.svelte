<script lang="ts">
  interface Props {
    title: string;
    subtitle?: string;
    dark?: boolean;
    center?: boolean;
  }

  let { title, subtitle, dark = false, center = true }: Props = $props();
</script>

<div class="{center ? 'text-center' : ''} mb-16">
  <h2 class="text-4xl md:text-5xl font-extrabold tracking-tight {dark ? 'text-heading-dark' : 'text-heading'}">
    {title}
  </h2>
  {#if subtitle}
    <p class="mt-4 text-lg {dark ? 'text-body-dark' : 'text-body'} max-w-2xl {center ? 'mx-auto' : ''}">
      {subtitle}
    </p>
  {/if}
</div>
