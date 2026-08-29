<script lang="ts">
  import { icons, type IconName } from "$lib/icons";

  interface Props {
    name: IconName;
    class?: string;
    label?: string;
  }

  let { name, class: cls = "h-4 w-4", label }: Props = $props();
  const def = $derived(icons[name]);
</script>

<svg
  class={cls}
  viewBox={def.viewBox}
  fill={def.mode === "stroke" ? "none" : undefined}
  stroke={def.mode === "stroke" ? "currentColor" : "none"}
  stroke-width={def.mode === "stroke" ? "2" : undefined}
  aria-hidden={label ? undefined : "true"}
  role={label ? "img" : undefined}
  aria-label={label}
>
  {@html def.html}
</svg>
