<script lang="ts">
  import type {ImagePackageInventory} from '@capsem/sdk';
  let {inventory}:{inventory:ImagePackageInventory}=$props();
  let search=$state('');
  let query=$derived(search.trim().toLocaleLowerCase());
  let packages=$derived(inventory.packages.filter(row=>`${row.name} ${row.version}`.toLocaleLowerCase().includes(query)));
  let tools=$derived(inventory.tools.filter(row=>`${row.name} ${row.version}`.toLocaleLowerCase().includes(query)));
</script>
<div class="mt-3 space-y-3">
  <label class="block text-xs text-muted-foreground-1">Find a package or tool
    <input type="search" bind:value={search} class="mt-1 block w-full rounded-md border border-line-2 bg-layer px-2 py-1.5 text-foreground" />
  </label>
  {#each [{label:'Shared tools',rows:tools},{label:'System packages',rows:packages}] as group}
    <section aria-label={group.label}>
      <h5 class="text-xs font-medium mb-1">{group.label} ({group.rows.length})</h5>
      <table class="w-full table-fixed text-xs">
        <thead><tr class="text-muted-foreground-1"><th class="text-start font-normal w-2/5 py-1">Name</th><th class="text-start font-normal py-1">Version</th></tr></thead>
        <tbody>{#each group.rows.slice(0,100) as row}<tr class="border-t border-card-line"><td class="align-top break-words py-1.5 pe-2">{row.name}</td><td class="align-top break-all py-1.5 font-mono">{row.version}</td></tr>{/each}</tbody>
      </table>
      {#if group.rows.length>100}<p class="text-xs text-muted-foreground-1">Showing 100 of {group.rows.length}. Refine the search to find more.</p>{/if}
    </section>
  {/each}
  <p class="text-xs text-muted-foreground-1">Debian system packages and verified shared tools in the selected image, before session changes.</p>
</div>
