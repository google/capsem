<script lang="ts">
  import {onMount} from 'svelte';
  import {ImageEvidenceKind,type ImageInfo} from '@capsem/sdk';
  import {imageDisplayName,safeProductUrl} from '../../models/image-metadata';
  import {openUrl} from '../../api';
  import AppIcon from './AppIcon.svelte';
  import PackageInventory from './PackageInventory.svelte';
  import X from 'phosphor-svelte/lib/X';
  let {image,onclose}:{image:ImageInfo;onclose:()=>void}=$props();
  let dialog:HTMLDialogElement;
  let error=$state<string|null>(null);
  let copied=$state<string|null>(null);
  let productUrl=$derived(safeProductUrl(image.app?.url));
  const artifacts=[{kind:ImageEvidenceKind.OBOM,label:'OBOM'},{kind:ImageEvidenceKind.SBOM,label:'SBOM'},
    {kind:ImageEvidenceKind.ABOM,label:'ABOM'},{kind:ImageEvidenceKind.PACKAGE_INVENTORY,label:'Installed packages and versions'}];
  onMount(()=>{dialog.showModal();});
  async function openProduct(){if(!productUrl)return;try{await openUrl(productUrl);}catch{error='Could not open the official website.';}}
  async function copy(reference:string){try{await navigator.clipboard.writeText(reference);copied=reference;}catch{error='Could not copy the artifact reference.';}}
</script>
<dialog bind:this={dialog} onclose={onclose} aria-labelledby="image-info-title"
  class="fixed inset-y-0 start-auto end-0 m-0 h-dvh max-h-none w-full max-w-lg border-s border-overlay-border bg-overlay text-foreground p-0 shadow-xl backdrop:bg-overlay/70">
  <div class="flex items-center gap-3 border-b border-overlay-border p-4">
    <AppIcon icon={image.app?.icon} />
    <h2 id="image-info-title" class="min-w-0 flex-1 text-lg font-medium">{imageDisplayName(image)}</h2>
    <button type="button" aria-label="Close app information" class="rounded-lg p-2 text-muted-foreground-1 hover:bg-muted-hover" onclick={()=>dialog.close()}><X size={18}/></button>
  </div>
  <div class="space-y-5 p-5 text-sm">
    <p>{image.app?.description ?? image.description}</p>
    <dl class="space-y-3">
      <div><dt class="text-xs text-muted-foreground-1">Publisher</dt><dd>{image.app?.publisher ?? 'Not provided by this catalog'}</dd></div>
      <div><dt class="text-xs text-muted-foreground-1">Official website</dt><dd>{#if productUrl}<button type="button" class="text-primary hover:underline break-all text-start" onclick={openProduct}>{productUrl}</button>{:else}Not provided by this catalog{/if}</dd></div>
      <div><dt class="text-xs text-muted-foreground-1">Image</dt><dd class="break-all text-xs font-mono">{image.app?.image ?? image.image ?? 'Unavailable for this computer'}</dd></div>
      <div><dt class="text-xs text-muted-foreground-1">Architecture</dt><dd>{image.app?.architecture ?? 'Not provided by this catalog'}</dd></div>
    </dl>
    <section aria-label="Image evidence" class="space-y-3">
      <h3 class="font-medium">Image evidence</h3>
      <p class="text-xs text-muted-foreground-1">References apply to this exact image and architecture.</p>
      {#each artifacts as artifact}
        {@const evidence=image.app?.evidence?.find(item=>item.kind===artifact.kind)}
        <div class="rounded-lg border border-card-line p-3">
          <h4 class="text-xs font-medium">{artifact.label}</h4>
          {#if evidence}
            <p class="mt-1 break-all text-xs font-mono text-muted-foreground-1">{evidence.reference}</p>
            <button type="button" class="mt-2 text-xs text-primary hover:underline" onclick={()=>copy(evidence.reference)}>{copied===evidence.reference?'Copied':'Copy artifact reference'}</button>
            {#if artifact.kind===ImageEvidenceKind.PACKAGE_INVENTORY}<PackageInventory {image}/>{/if}
          {:else}<p class="mt-1 text-xs text-muted-foreground-1">{image.app ? 'Not published for this image.' : 'Not provided by this service.'}</p>{/if}
        </div>
      {/each}
    </section>
    {#if error}<p role="alert" class="text-xs text-destructive">{error}</p>{/if}
  </div>
</dialog>
