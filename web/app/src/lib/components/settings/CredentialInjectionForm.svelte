<script lang="ts">
  import {CredentialInjectProvider, CredentialStorage} from '@capsem/sdk';
  import {injectCredential} from '../../api';

  let {onadded}: {onadded: () => void} = $props();
  let provider = $state(CredentialInjectProvider.OPENAI);
  let storage = $state(CredentialStorage.FILE);
  let value = $state('');
  let saving = $state(false);
  let reference = $state('');
  let error = $state('');
  let copied = $state(false);

  const providers = [
    {value: CredentialInjectProvider.OPENAI, label: 'OpenAI'},
    {value: CredentialInjectProvider.ANTHROPIC, label: 'Anthropic'},
    {value: CredentialInjectProvider.GOOGLE, label: 'Google'},
    {value: CredentialInjectProvider.GITHUB, label: 'GitHub'},
  ];

  function clearFeedback() {
    reference = '';
    error = '';
    copied = false;
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (saving || !value) return;
    saving = true;
    error = '';
    reference = '';
    copied = false;
    const secret = value;
    value = '';
    try {
      const result = await injectCredential(provider, secret, storage);
      reference = result.credential_ref;
      onadded();
    } catch {
      // Provider/transport errors may contain submitted material. Keep them
      // out of the UI, logs and browser persistence; retry is always explicit.
      error = 'The service could not confirm the credential. Enter it again to retry.';
    } finally {
      saving = false;
    }
  }

  async function copyReference() {
    try {
      await navigator.clipboard.writeText(reference);
      copied = true;
    } catch {
      error = 'Could not copy the reference. Select it to copy manually.';
    }
  }
</script>

<form class="mt-4 border-t border-card-line pt-4" onsubmit={submit} autocomplete="off">
  <p class="text-sm font-medium text-foreground">Add a credential</p>
  <div class="grid gap-3 sm:grid-cols-2 mt-3">
    <label class="text-xs text-muted-foreground-1">
      Provider
      <select bind:value={provider} onchange={clearFeedback} disabled={saving} class="mt-1 block w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:border-primary disabled:opacity-60">
        {#each providers as item (item.value)}<option value={item.value}>{item.label}</option>{/each}
      </select>
    </label>
    <label class="text-xs text-muted-foreground-1">
      Storage
      <select bind:value={storage} onchange={clearFeedback} disabled={saving} class="mt-1 block w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:border-primary disabled:opacity-60">
        <option value={CredentialStorage.FILE}>File — keep for future starts</option>
        <option value={CredentialStorage.MEMORY}>Memory — until the service exits</option>
      </select>
    </label>
  </div>
  <label class="block mt-3 text-xs text-muted-foreground-1">
    API key or token
    <input type="password" bind:value oninput={clearFeedback} autocomplete="off" spellcheck={false} disabled={saving} class="mt-1 block w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:border-primary disabled:opacity-60" />
  </label>
  <button type="submit" disabled={saving || !value} class="mt-3 py-2 px-3 text-xs font-medium rounded-md bg-primary text-primary-foreground hover:bg-primary-hover disabled:opacity-60">
    {saving ? 'Adding…' : 'Add credential'}
  </button>
  {#if error}<p class="mt-3 text-xs text-destructive-text" role="alert">{error}</p>{/if}
  {#if reference}
    <div class="mt-3 text-xs text-muted-foreground-1" role="status">
      <p>Credential added. Use this reference when configuring a VM.</p>
      <div class="flex flex-wrap items-center gap-2 mt-2">
        <code class="break-all select-all text-foreground">{reference}</code>
        <button type="button" onclick={copyReference} class="py-1.5 px-3 rounded-md bg-muted text-foreground hover:bg-muted-hover">{copied ? 'Copied' : 'Copy reference'}</button>
      </div>
    </div>
  {/if}
</form>
