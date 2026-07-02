import { existsSync, readFileSync } from 'node:fs';
import { isAbsolute, resolve } from 'node:path';

type JsonObject = Record<string, any>;

export interface ReleaseData {
  dist: string;
  channel: string;
  channels: JsonObject;
  channelRecord: JsonObject;
  manifestRecord: JsonObject;
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
  const dist = resolveReleaseDist(distEnv);
  const channels = readJson(resolve(dist, 'channels.json'));
  const channel = selectChannel(channels);
  const channelRecord = channels.channels?.[channel] ?? {};
  const manifestRecord = selectManifestRecord(channelRecord);
  const manifestPath = trimLeadingSlash(String(manifestRecord.url ?? `/assets/${channel}/manifest.json`));
  const manifest = readJson(resolve(dist, manifestPath));
  const catalogPath = trimLeadingSlash(String(channelRecord.profile_catalog?.source ?? ''));
  const catalog = readJson(resolve(dist, catalogPath));
  return { dist, channel, channels, channelRecord, manifestRecord, manifest, catalog };
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
  const current = String(data.manifest.binaries?.current ?? '');
  const files = data.manifest.binaries?.releases?.[current]?.files;
  return Array.isArray(files) ? files.filter((file: JsonObject) => !isHostSbom(file.name)) : [];
}

export function hostSbomRows(data: ReleaseData): JsonObject[] {
  const current = String(data.manifest.binaries?.current ?? '');
  const files = data.manifest.binaries?.releases?.[current]?.files;
  return Array.isArray(files) ? files.filter((file: JsonObject) => isHostSbom(file.name)) : [];
}

export function vmObomRows(data: ReleaseData): JsonObject[] {
  return profileList(data)
    .flatMap((profile) => {
      const obomByArch = profile.obom?.arch ?? {};
      return Object.entries(obomByArch).map(([arch, descriptor]) => ({
        arch,
        ...(descriptor as JsonObject),
      }));
    })
    .sort((left, right) => String(left.arch).localeCompare(String(right.arch)));
}

export function currentAssetFilesByArch(data: ReleaseData): [string, JsonObject[]][] {
  const current = String(data.manifest.assets?.current ?? '');
  const release = data.manifest.assets?.releases?.[current] ?? {};
  const arches = release.arches ?? {};
  const assetBase = currentAssetBaseUrl(data);
  const files = Object.entries(arches).flatMap(([arch, entries]) => {
    return Object.entries(entries as JsonObject).map(([logicalName, entry]) => ({
      arch,
      logical_name: logicalName,
      url: assetFileUrl(assetBase, arch, logicalName),
      ...(entry as JsonObject),
    }));
  });
  const grouped = new Map<string, JsonObject[]>();
  for (const file of files) {
    const arch = String(file.arch ?? 'unknown');
    const rows = grouped.get(arch) ?? [];
    rows.push(file);
    grouped.set(arch, rows);
  }
  return Array.from(grouped.entries())
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([arch, rows]) => [
      arch,
      rows.sort((left, right) => String(left.logical_name ?? '').localeCompare(String(right.logical_name ?? ''))),
    ]);
}

export function assetReleaseRows(data: ReleaseData): JsonObject[] {
  const releases = data.manifest.assets?.releases ?? {};
  return Object.entries(releases)
    .map(([version, release]) => {
      const item = release as JsonObject;
      return {
        version,
        date: item.date,
        state: item.deprecated ? 'deprecated' : 'current',
        deprecated: Boolean(item.deprecated),
        deprecated_date: item.deprecated_date,
        min_binary: item.min_binary,
        arches: Object.keys(item.arches ?? {}).sort(),
      };
    })
    .sort((left, right) => String(right.version).localeCompare(String(left.version)));
}

export function currentArchitectures(data: ReleaseData): string[] {
  return Object.keys(
    data.manifest.assets?.releases?.[data.manifest.assets?.current]?.arches ?? {},
  ).sort();
}

export function currentAssetBaseUrl(data: ReleaseData): string {
  const template = String(data.manifestRecord.asset_base ?? data.manifest.asset_base ?? '/assets/releases');
  const assetVersion = String(data.manifest.assets?.current ?? '');
  if (template.includes('{asset_version}')) {
    return template.replace('{asset_version}', assetVersion);
  }
  if (template.replace(/\/+$/, '').endsWith('/assets/releases')) {
    return `${template.replace(/\/+$/, '')}/${assetVersion}`;
  }
  return template;
}

export function generatedAt(data: ReleaseData): string {
  return String(data.channels.generated_at ?? '');
}

export function profileCatalogUrl(data: ReleaseData): string {
  return String(data.channelRecord.profile_catalog?.source ?? '');
}

export function profileRevision(data: ReleaseData): string {
  return String(data.catalog.revision ?? data.channelRecord.profile_catalog?.revision ?? '');
}

export function manifestUrl(data: ReleaseData): string {
  return String(data.manifestRecord.url ?? `/assets/${data.channel}/manifest.json`);
}

export function manifestBlake3(data: ReleaseData): string {
  return String(data.manifestRecord.digest?.blake3 ?? '');
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

function selectChannel(channels: JsonObject): string {
  const entries = channels.channels ?? {};
  if (entries.stable) {
    return 'stable';
  }
  const first = Object.keys(entries).sort()[0];
  if (!first) {
    throw new Error('channels.json must list at least one channel');
  }
  return first;
}

function selectManifestRecord(channelRecord: JsonObject): JsonObject {
  const manifests = Array.isArray(channelRecord.manifests) ? channelRecord.manifests : [];
  const selected = manifests.find((manifest: JsonObject) => manifest.status === 'current')
    ?? manifests.find((manifest: JsonObject) => manifest.status === 'supported')
    ?? manifests.find((manifest: JsonObject) => manifest.status === 'deprecated');
  if (!selected) {
    throw new Error('channels.json channel must list a selectable manifest');
  }
  return selected;
}

function assetFileUrl(assetBase: string, arch: string, logicalName: string): string {
  const normalizedBase = assetBase.replace(/\/+$/, '');
  return `${normalizedBase}/${arch}-${logicalName}`;
}

function readJson(path: string): JsonObject {
  if (!existsSync(path)) {
    throw new Error(`Release-site input is missing: ${path}`);
  }
  return JSON.parse(readFileSync(path, 'utf8')) as JsonObject;
}

function resolveReleaseDist(path: string): string {
  if (isAbsolute(path)) {
    return path;
  }
  const fromCwd = resolve(process.cwd(), path);
  if (existsSync(fromCwd)) {
    return fromCwd;
  }
  return resolve(process.cwd(), '..', path);
}

function trimLeadingSlash(path: string): string {
  return path.replace(/^\/+/, '');
}

function isHostSbom(name: unknown): boolean {
  return name === 'capsem-sbom.spdx.json';
}
