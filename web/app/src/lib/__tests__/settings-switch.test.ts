import {expect,it} from 'vitest';
import {render} from 'svelte/server';
import Switch from '../components/settings/widgets/Switch.svelte';
it('exposes an accessible off switch with the shared pill presentation',()=>{
  const html=render(Switch,{props:{label:'Developer mode',checked:false,onchange:()=>{}}}).body;
  expect(html).toContain('role="switch"');
  expect(html).toContain('aria-label="Developer mode"');
  expect(html).toContain('aria-checked="false"');
  expect(html).toContain('rounded-full');
});
it('exposes checked and disabled states without a checkbox appearance',()=>{
  const html=render(Switch,{props:{label:'Developer mode',checked:true,disabled:true,onchange:()=>{}}}).body;
  expect(html).toContain('aria-checked="true"');
  expect(html).toContain('disabled');
  expect(html).not.toContain('type="checkbox"');
});
