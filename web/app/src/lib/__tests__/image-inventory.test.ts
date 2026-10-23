import {expect,it,vi} from 'vitest';
import type {ImageInventoryResponse} from '@capsem/sdk';
import {ImageInventoryReader} from '../models/image-inventory';
import type {InventoryView} from '../models/image-inventory';
const pin='registry.example/dev@sha256:'+'1'.repeat(64);
const response:ImageInventoryResponse={image:pin,architecture:'arm64',artifact_reference:'registry.example/dev@sha256:'+'2'.repeat(64),
 inventory:{schema:'capsem.package_inventory.v1',subject:pin,architecture:'arm64',packages:[{name:'git',version:'2.39.5',architecture:'arm64'}],tools:[{name:'gh',version:'2.23.0'}]}};
it('keeps failed reads distinct from an explicitly unavailable inventory',async()=>{
 const updates:InventoryView[]=[];
 const read=vi.fn().mockRejectedValueOnce(Error('verification failed')).mockResolvedValueOnce({image:pin,architecture:'arm64'});
 const owner=new ImageInventoryReader(read,state=>updates.push(state));
 await owner.load('dev',pin,'arm64');
 expect(updates.at(-1).status).toBe('error');
 await owner.load('dev',pin,'arm64');
 expect(updates.at(-1).status).toBe('unavailable');
});
it('refuses different image or platform responses',async()=>{
 for(const data of [{...response,image:'different'},{...response,architecture:'amd64'},{...response,artifact_reference:null}]) {
  const updates:InventoryView[]=[];
  const owner=new ImageInventoryReader(async()=>data,state=>updates.push(state));
  await owner.load('dev',pin,'arm64');
  expect(updates.at(-1).status).toBe('error');
 }
});
it('aborts on close and ignores late responses',async()=>{
 let complete!:(value:ImageInventoryResponse)=>void;
 const read=vi.fn((_name:string,_pin:string,_signal:AbortSignal)=>new Promise<ImageInventoryResponse>(resolve=>{complete=resolve;}));
 const updates:InventoryView[]=[];
 const owner=new ImageInventoryReader(read,state=>updates.push(state));
 const loading=owner.load('dev',pin,'arm64');
 owner.dispose();
 expect(read.mock.calls[0][2].aborted).toBe(true);
 complete(response);await loading;
 expect(updates.at(-1).status).toBe('loading');
});
it('retains verified package data from the exact requested image',async()=>{
 const updates:InventoryView[]=[];
 const owner=new ImageInventoryReader(async()=>response,state=>updates.push(state));
 await owner.load('dev',pin,'arm64');
 expect(updates.at(-1)).toEqual({status:'ready',inventory:response.inventory});
});
