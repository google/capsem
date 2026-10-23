import { beforeEach, expect, it, vi } from 'vitest';
const values=new Map<string,string>();
vi.stubGlobal('localStorage',{getItem:(key:string)=>values.get(key)??null,setItem:(key:string,value:string)=>values.set(key,value)});
beforeEach(()=>{ values.clear(); vi.resetModules(); });
it('defaults developer mode off and persists explicit opt-in',async()=>{
  const {developerStore}=await import('../stores/developer.svelte');
  expect(developerStore.enabled).toBe(false);
  developerStore.setEnabled(true);
  expect(values.get('capsem-developer-mode')).toBe('true');
  vi.resetModules();
  expect((await import('../stores/developer.svelte')).developerStore.enabled).toBe(true);
});
it('does not opt in from malformed preference',async()=>{
  values.set('capsem-developer-mode','yes');
  expect((await import('../stores/developer.svelte')).developerStore.enabled).toBe(false);
});
