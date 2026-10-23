import {expect,it} from 'vitest';
import {ImageCacheState,ImageSurface} from '@capsem/sdk';
import {safeProductUrl,safeProductIcon,imageDisplayName,catalogGroups} from '../image-metadata';
it('uses canonical display metadata and never guesses names',()=>{
 const image={name:'dev',description:'Tools',architectures:['arm64'],cached:ImageCacheState.MISSING};
 expect(imageDisplayName(image)).toBe('dev');
 expect(imageDisplayName({...image,app:{catalog_name:'dev',image:'registry/dev@sha256:a',display_name:'Development',description:'Tools',architecture:'arm64',interfaces:[],evidence:[]}})).toBe('Development');
});
it('only exposes safe public product URLs and actual icon references',()=>{
 for(const value of ['javascript:alert(1)','data:image/svg+xml,<svg/>','https://user:secret@example.com/icon.svg','//example.com/icon.svg']){
  expect(safeProductUrl(value)).toBeNull(); expect(safeProductIcon(value)).toBeNull();
 }
 expect(safeProductUrl('https://example.com/product')).toBe('https://example.com/product');
 expect(safeProductIcon('/icons/products/app.svg')).toBe('/icons/products/app.svg');
 expect(safeProductIcon('app.svg')).toBeNull();
 expect(safeProductIcon('/icons/../secret.svg')).toBeNull();
});
it('groups only by the authoritative execution surface and preserves unknowns',()=>{
 const image={name:'app',description:'Tools',architectures:[],cached:ImageCacheState.MISSING};
 const groups=catalogGroups([{...image,name:'gui',surface:ImageSurface.XPRA},{...image,name:'cli',surface:ImageSurface.TERMINAL},{...image,name:'unknown'}]);
 expect(groups.map(group=>[group.label,group.images.map(image=>image.name)])).toEqual([['Graphical',['gui']],['Terminal',['cli']],['Other',['unknown']]]);
});
