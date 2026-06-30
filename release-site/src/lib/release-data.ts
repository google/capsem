import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';

type JsonObject = Record<string, any>;

export interface ReleaseData {
  dist: string;
  channel: string;
  health: JsonObject;
  manifest: JsonObject;
  catalog: JsonObject;
}

export interface TableRow {
  label: string;
  name: string;
  url?: string;
  size?: number | string;
  hash?: string;
  status?: string;
}

export function loadReleaseData(): ReleaseData {
  const distEnv = process.env.CAPSEM_RELEASE_CHANNEL_DIST;
  if (!distEnv) {
    throw new Error('CAPSEM_RELEASE_CHANNEL_DIST must point at target/release-channel');
  }
  const dist = resolve(process.cwd(), distEnv);
  const health = readJson(resolve(dist, 'health.json'));
  const channel = String(health.channel ?? 'stable');
  const manifestPath = trimLeadingSlash(String(health.urls?.manifest ?? `/assets/${channel}/manifest.json`));
  const manifest = readJson(resolve(dist, manifestPath));
  const catalogPath = trimLeadingSlash(String(health.profiles?.source ?? health.urls?.profile_catalog ?? ''));
  const catalog = readJson(resolve(dist, catalogPath));
  return { dist, channel, health, manifest, catalog };
}

export function profilePagePath(profileId: string): string {
  return `/profiles/${encodeURIComponent(profileId)}/`;
}

export function profileList(data: ReleaseData): JsonObject[] {
  return Array.isArray(data.catalog.profiles) ? data.catalog.profiles : [];
}

export function profileById(data: ReleaseData, id: string): JsonObject | undefined {
  return profileList(data).find((profile) => profile.id === id);
}

export function profileArchNames(profile: JsonObject): string[] {
  return Object.keys(profile.assets?.arch ?? {}).sort();
}

export function profileArtifactRows(profile: JsonObject, arch: string): TableRow[] {
  const assets = profile.assets?.arch?.[arch] ?? {};
  const rows: TableRow[] = [];
  for (const [key, label] of [
    ['kernel', 'Kernel'],
    ['initrd', 'Initrd'],
    ['rootfs', 'Root filesystem'],
  ] as const) {
    const descriptor = assets[key];
    if (descriptor) {
      rows.push(descriptorRow(label, descriptor));
    }
  }

  const abom = profile.abom?.arch?.[arch] ?? profile.obom?.arch?.[arch];
  if (abom) {
    rows.push(descriptorRow('ABOM / OBOM', abom));
  } else {
    rows.push({ label: 'ABOM / OBOM', name: 'Not published in profile catalog', status: 'missing' });
  }

  const sbom = profile.sbom?.arch?.[arch];
  if (sbom) {
    rows.push(descriptorRow('SBOM', sbom));
  } else {
    rows.push({ label: 'SBOM', name: 'Not published in profile catalog', status: 'missing' });
  }
  return rows;
}

export function profileFileRows(profile: JsonObject): TableRow[] {
  return Object.entries(profile.files ?? {}).map(([kind, descriptor]) => {
    const item = descriptor as JsonObject;
    return {
      label: kind,
      name: String(item.path ?? ''),
      size: item.size,
      hash: item.hash,
    };
  });
}

export function binaryRows(data: ReleaseData): JsonObject[] {
  return (data.health.binary?.files ?? []).filter((file: JsonObject) => !isHostSbom(file.name));
}

export function hostSbomRows(data: ReleaseData): JsonObject[] {
  return data.health.evidence?.host_sboms ?? [];
}

export function vmObomRows(data: ReleaseData): JsonObject[] {
  return data.health.evidence?.vm_oboms ?? [];
}

export function assetReleaseRows(data: ReleaseData): JsonObject[] {
  return data.health.asset_releases ?? [];
}

export function currentArchitectures(data: ReleaseData): string[] {
  return Object.keys(
    data.manifest.assets?.releases?.[data.manifest.assets?.current]?.arches ?? {},
  ).sort();
}

export function currentAssetBaseUrl(data: ReleaseData): string {
  const template = String(data.health.urls?.asset_base ?? '');
  const assetVersion = String(data.health.current?.assets ?? data.manifest.assets?.current ?? '');
  return template.replace('{asset_version}', assetVersion);
}

export function byteLabel(value: unknown): string {
  return typeof value === 'number' ? value.toLocaleString('en-US') : 'unknown';
}

export function hashLabel(value: unknown): string {
  return typeof value === 'string' && value.length > 0 ? value : 'not published';
}

export function binaryDescription(name: string): string {
  if (name.endsWith('.pkg')) return 'macOS installer package';
  if (name.endsWith('.deb')) return 'Linux Debian package';
  return 'Capsem binary package';
}

function descriptorRow(label: string, descriptor: JsonObject): TableRow {
  return {
    label,
    name: String(descriptor.name ?? ''),
    url: descriptor.url,
    size: descriptor.size,
    hash: descriptor.hash,
  };
}

function readJson(path: string): JsonObject {
  if (!existsSync(path)) {
    throw new Error(`Release-site input is missing: ${path}`);
  }
  return JSON.parse(readFileSync(path, 'utf8')) as JsonObject;
}

function trimLeadingSlash(path: string): string {
  return path.replace(/^\/+/, '');
}

function isHostSbom(name: unknown): boolean {
  return name === 'capsem-sbom.spdx.json';
}
