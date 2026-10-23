import {expect,it} from 'vitest';
import {render} from 'svelte/server';
import type {ImagePackageInventory} from '@capsem/sdk';
import Table from '../components/shell/InventoryTable.svelte';
it('renders real package and command versions as text, with bounded initial rows',()=>{
 const inventory:ImagePackageInventory={schema:'capsem.package_inventory.v1',subject:'exact-image',architecture:'arm64',
   tools:[{name:'gh',version:'2.23.0'}],packages:Array.from({length:101},(_,index)=>({name:index===0?'<script>unsafe</script>':`package-${index}`,version:'1.2.3',architecture:'all'}))};
 const html=render(Table,{props:{inventory}}).body;
 expect(html).toContain('gh');expect(html).toContain('2.23.0');
 expect(html).toContain('&lt;script>unsafe&lt;/script>');
 expect(html).not.toContain('<script>unsafe');
 expect(html).toContain('Showing 100 of 101');
 expect(html).not.toContain('package-100');
});
