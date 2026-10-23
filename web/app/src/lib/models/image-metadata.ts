import {ImageSurface,type ImageInfo} from '@capsem/sdk';

export function safeProductUrl(value: string | null | undefined): string | null {
  if (!value) return null;
  try {
    const url=new URL(value);
    return url.protocol==='https:' && !url.username && !url.password ? url.href : null;
  } catch { return null; }
}
export function safeProductIcon(value: string | null | undefined): string | null {
  if (value?.startsWith('/icons/') && /^\/icons\/[a-zA-Z0-9_./-]+$/.test(value) && !value.includes('..')) return value;
  return safeProductUrl(value);
}
export function imageDisplayName(image: ImageInfo): string { return image.app?.display_name || image.name; }
export function catalogGroups(images: readonly ImageInfo[]): {label:string;images:ImageInfo[]}[] {
  const groups=[{label:'Graphical',images:images.filter(image=>image.surface===ImageSurface.XPRA)},
    {label:'Terminal',images:images.filter(image=>image.surface===ImageSurface.TERMINAL)},
    {label:'Other',images:images.filter(image=>!image.surface)}];
  return groups.filter(group=>group.images.length>0);
}
