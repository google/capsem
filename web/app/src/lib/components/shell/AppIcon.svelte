<script lang="ts">
  import {safeProductIcon} from '../../models/image-metadata';
  import AppWindow from 'phosphor-svelte/lib/AppWindow';
  let {icon, class:cls='size-8'}:{icon?:string|null;class?:string}=$props();
  let failed=$state(false);
  let source=$derived(safeProductIcon(icon));
  $effect(()=>{void source;failed=false;});
</script>
<span class="{cls} shrink-0 inline-flex items-center justify-center text-muted-foreground-1">
  {#if source && !failed}<img src={source} alt="" class="size-full object-contain" referrerpolicy="no-referrer" onerror={()=>failed=true} />
  {:else}<AppWindow size={22} aria-label="Product icon unavailable" />{/if}
</span>
