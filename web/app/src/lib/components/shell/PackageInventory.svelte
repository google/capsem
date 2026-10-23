<script lang="ts">
  import {onMount} from 'svelte';
  import type {ImageInfo} from '@capsem/sdk';
  import {getImageInventory} from '../../api';
  import {ImageInventoryReader,type InventoryView} from '../../models/image-inventory';
  import InventoryTable from './InventoryTable.svelte';
  let {image}:{image:ImageInfo}=$props();
  let view=$state<InventoryView>({status:'loading'});
  const reader=new ImageInventoryReader(getImageInventory,next=>{view=next;});
  function load(){if(image.image&&image.app)void reader.load(image.name,image.image,image.app.architecture);}
  onMount(()=>{load();return()=>reader.dispose();});
</script>
{#if view.status==='loading'}<p role="status" class="mt-3 text-xs text-muted-foreground-1">Reading package inventory…</p>
{:else if view.status==='error'}<div class="mt-3"><p role="alert" class="text-xs text-destructive">Could not verify or read this image’s inventory.</p><button type="button" class="mt-2 text-xs text-primary hover:underline" onclick={load}>Retry inventory</button></div>
{:else if view.status==='unavailable'}<p class="mt-3 text-xs text-muted-foreground-1">No package inventory is provided by this catalog.</p>
{:else}<InventoryTable inventory={view.inventory}/>{/if}
