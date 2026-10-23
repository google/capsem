import {expect,it,vi} from 'vitest';
import {render} from 'svelte/server';
import {ImageCacheState,type ImageInfo} from '@capsem/sdk';
vi.mock('../api',()=>({openUrl:vi.fn(),getImageInventory:vi.fn()}));
import Panel from '../components/shell/ImageInfoPanel.svelte';
const image:ImageInfo={name:'dev',description:'Tools',architectures:['arm64'],cached:ImageCacheState.MISSING,image:'registry.example/dev@sha256:a'};
it('does not call unknown older-service artifact metadata unpublished',()=>{
 const html=render(Panel,{props:{image,onclose:()=>{}}}).body;
 expect(html).toContain('Not provided by this service.');
 expect(html).not.toContain('Not published for this image.');
});
it('explicitly identifies missing artifacts only when canonical metadata is present',()=>{
 const html=render(Panel,{props:{image:{...image,app:{catalog_name:'dev',image:image.image!,display_name:'Development',description:'Tools',architecture:'arm64',interfaces:[],evidence:[]}},onclose:()=>{}}}).body;
 expect(html).toContain('Development');
 expect(html).toContain('Not published for this image.');
});
