import { expect, it } from 'vitest';
import { render } from 'svelte/server';
import type { CredentialBrokerInfo } from '../api';
import List from '../components/settings/CredentialList.svelte';
const row = { provider:'google',credential_ref:'credential:private-reference',observed_count:2,injected_count:1,replay_available:true,last_seen:'2026-10-23T12:00:00Z' };
const info: CredentialBrokerInfo = { plugin_id:'credential_broker',inventory:[row],corp_constraints:[],
  grants:{enabled:true,vm_grants:[],fork_default:'inherit'},
  store:{backend:'test',ready:true,status:'ready',cached_count:1,last_hydrated_count:1,last_hydrated_unix_ms:null,last_error:null} };
it('shows only read-only metadata with authoritative VM association and explicit missing source',()=>{
  const html = render(List,{props:{info:{...info,sessions:[{vm_id:'vm-a',inventory:[row]}]}}}).body;
  for (const text of ['vm-a','google','Not recorded','2026-10-23T12:00:00Z']) expect(html).toContain(text);
  for (const text of ['<button','<input','<select','private-reference','Grant access','Inject','Connect account','Retry store']) expect(html).not.toContain(text);
  expect(html).not.toContain('VM association unavailable');
});
it('never assigns global inventory to every VM and labels unknown associations',()=>{
  const html = render(List,{props:{info:{...info,sessions:[{vm_id:'vm-empty',inventory:[]}]}}}).body;
  expect(html).toContain('No credential activity recorded for this VM.');
  expect(html).toContain('VM association unavailable');
  expect(html.match(/google/g)).toHaveLength(1);
});
it('escapes metadata and keeps older service association unavailable',()=>{
  const html = render(List,{props:{info:{...info,inventory:[{...row,provider:'<script>unsafe</script>'}]}}}).body;
  expect(html).toContain('VM association unavailable');
  expect(html).toContain('&lt;script>unsafe&lt;/script>');
  expect(html).not.toContain('<script>unsafe');
});
