import {expect,it,vi} from 'vitest';
import {render} from 'svelte/server';
import Controls from '../components/settings/ContentPluginControls.svelte';
const content={direction:'both' as const,email:true,phone:true,card:true,ssn:true};
it('presents independent bounded HTTP PII types with explicit heuristic scope',()=>{
  const html=render(Controls,{props:{pluginId:'pii_filter',content,disabled:false,onchange:vi.fn()}}).body;
  for(const label of ['Email addresses','Phone numbers','Payment cards','US Social Security numbers','Inbound','Outbound','heuristic','16 MiB','32 MiB','60 seconds'])expect(html).toContain(label);
  expect(html.match(/type="checkbox"/g)).toHaveLength(4);
});
it('keeps secret scanning outbound only and Unicode validation separate',()=>{
  const secrets=render(Controls,{props:{pluginId:'secret_exfiltration',content,disabled:false,onchange:vi.fn()}}).body;
  expect(secrets).toContain('Outbound only');expect(secrets).not.toContain('type="checkbox"');
  expect(secrets).toContain('broker credential references');
  const utf8=render(Controls,{props:{pluginId:'utf8_guard',content,disabled:false,onchange:vi.fn()}}).body;
  expect(utf8).toContain('Valid Unicode');expect(utf8).not.toContain('type="checkbox"');
});
