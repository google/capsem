<script lang="ts">
  import type {ContentFilterConfig} from '../../api';
  let {pluginId,content,disabled,onchange}:{pluginId:string;content:ContentFilterConfig;disabled:boolean;
    onchange:(content:ContentFilterConfig)=>void}=$props();
  const types:{key:'email'|'phone'|'card'|'ssn';label:string}[]=[
    {key:'email',label:'Email addresses'},{key:'phone',label:'Phone numbers (North America)'},
    {key:'card',label:'Payment cards (Luhn check)'},{key:'ssn',label:'US Social Security numbers'},
  ];
</script>
<div class="mt-4 space-y-3 border-t border-line-2 pt-3 text-sm">
  {#if pluginId==='secret_exfiltration'}
    <p class="font-medium">Outbound only</p>
    <p class="text-xs text-muted-foreground-1">Checks supported token/private-key patterns and broker credential references outside authorized authentication fields. Detection does not cover every secret or encrypted payload.</p>
  {:else}
    <label class="flex flex-wrap items-center gap-3"><span>Direction</span>
      <select aria-label="{pluginId} inspection direction" class="rounded-lg border border-line-2 bg-layer px-3 py-2 text-foreground disabled:opacity-60" {disabled}
        value={content.direction} onchange={event=>onchange({...content,direction:event.currentTarget.value as ContentFilterConfig['direction']})}>
        <option value="both">Inbound and outbound</option><option value="inbound">Inbound responses</option><option value="outbound">Outbound requests</option>
      </select>
    </label>
    {#if pluginId==='pii_filter'}
      <fieldset class="space-y-2"><legend class="mb-2 text-xs font-medium">Selected PII types</legend>
        {#each types as type}
          <label class="flex items-center gap-2"><input type="checkbox" class="accent-primary" {disabled} checked={content[type.key]}
            onchange={event=>onchange({...content,[type.key]:event.currentTarget.checked})}/><span>{type.label}</span></label>
        {/each}
      </fieldset>
      <p class="text-xs text-muted-foreground-1">Pattern-based heuristic detection can miss personal information or flag ordinary text. The plugin mode applies to all selected types.</p>
    {:else}
      <p class="text-xs text-muted-foreground-1">Valid Unicode text and binary media stay unchanged. Block rejects invalid UTF-8 text; there is no lossy text rewrite.</p>
    {/if}
  {/if}
  <p class="text-xs text-muted-foreground-1">HTTP inspection only. Enabled text inspection buffers complete bodies, including SSE: up to 16 MiB on the wire, 32 MiB after gzip decoding and 60 seconds. Unsupported encodings or protected upgrade paths fail closed; inspection can delay streaming.</p>
</div>
