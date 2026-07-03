import { existsSync, readFileSync, statSync } from 'node:fs';
import { isAbsolute, resolve } from 'node:path';

type JsonObject = Record<string, any>;
const DEFAULT_RELEASE_GRAPH = '../tests/capsem-release/fixtures/release-graph-stable-nightly.json';

export interface ReleaseData {
  dist: string;
  sourceMode: 'dist' | 'graph';
  graph?: JsonObject;
  channel: string;
  channels: JsonObject;
  channelRecord: JsonObject;
  manifestRecord: JsonObject;
  manifest: JsonObject;
  profileContract: JsonObject;
}

export interface TableRow {
  label: string;
  name: string;
  url?: string;
  size?: number | string;
  hash?: string;
  status?: string;
}

export interface ChannelRow {
  id: string;
  label: string;
  description: string;
  manifestCount: number;
  manifestRevision: string;
  currentStatus: string;
  statuses: string[];
  updatedAt: string;
  coverageLabel: string;
  manifestUrl: string;
  pageUrl: string;
}

export function loadReleaseData(): ReleaseData {
  const distEnv = process.env.CAPSEM_RELEASE_CHANNEL_DIST ?? DEFAULT_RELEASE_GRAPH;
  const dist = resolveReleaseInput(distEnv);
  if (isJsonFile(dist)) {
    return loadGraphData(dist);
  }
  const graphPath = resolve(dist, 'release-graph.json');
  if (!existsSync(resolve(dist, 'channels.json')) && existsSync(graphPath)) {
    return loadGraphData(graphPath);
  }
  return loadDistData(dist);
}

function loadDistData(dist: string): ReleaseData {
  const channels = readJson(resolve(dist, 'channels.json'));
  const channel = selectChannel(channels);
  const channelRecord = channels.channels?.[channel] ?? {};
  const manifestRecord = selectManifestRecord(channelRecord);
  const manifestPath = trimLeadingSlash(String(manifestRecord.url ?? `/assets/${channel}/manifest.json`));
  const manifest = readJson(resolve(dist, manifestPath));
  const profileContract = profileContractFromManifest(manifest);
  return { dist, sourceMode: 'dist', channel, channels, channelRecord, manifestRecord, manifest, profileContract };
}

function loadGraphData(graphPath: string): ReleaseData {
  const graph = readJson(graphPath);
  const channels = {
    version: graph.version ?? 1,
    generated_at: graph.generated_at ?? '',
    channels: graph.channels ?? {},
  };
  const channel = selectChannel(channels);
  const channelRecord = channels.channels?.[channel] ?? {};
  const manifestRecord = selectManifestRecord(channelRecord);
  const manifest = graph.manifests?.[channel]?.[manifestRecord.version];
  if (!manifest) {
    throw new Error(`Release graph is missing ${channel} manifest ${manifestRecord.version}`);
  }
  const profileContract = profileContractFromManifest(manifest);
  return {
    dist: graphPath,
    sourceMode: 'graph',
    graph,
    channel,
    channels,
    channelRecord,
    manifestRecord,
    manifest,
    profileContract,
  };
}

export function profilePagePath(profileId: string): string {
  return `/profiles/${encodeURIComponent(profileId)}/`;
}

export function channelProfilePagePath(channelId: string, profileId: string): string {
  return `/channels/${encodeURIComponent(channelId)}/profiles/${encodeURIComponent(profileId)}/`;
}

export function channelPackagePagePath(channelId: string, packageId: string): string {
  return `/channels/${encodeURIComponent(channelId)}/packages/${encodeURIComponent(packageId)}/`;
}

export function channelPagePath(channelId: string): string {
  return `/channels/${encodeURIComponent(channelId)}/`;
}

export function channelRows(data: ReleaseData): ChannelRow[] {
  return Object.entries(data.channels.channels ?? {})
    .map(([id, record]) => {
      const channel = record as JsonObject;
      const manifests = Array.isArray(channel.manifests) ? channel.manifests : [];
      const selected = selectManifestRecord(channel);
      const summary = selectedManifestSummary(data, id, selected);
      return {
        id,
        label: String(channel.label ?? id),
        description: String(channel.description ?? ''),
        manifestCount: manifests.length,
        manifestRevision: String(selected.revision ?? selected.version ?? 'not published'),
        currentStatus: String(selected.status ?? 'not published'),
        statuses: Array.from(new Set(manifests.map((manifest: JsonObject) => String(manifest.status ?? 'unknown')))),
        updatedAt: String(selected.updated_at ?? channel.updated_at ?? data.channels.generated_at ?? ''),
        coverageLabel: summary.coverageLabel,
        manifestUrl: String(selected.url ?? ''),
        pageUrl: channelPagePath(id),
      };
    })
    .sort((left, right) => left.id.localeCompare(right.id));
}

function selectedManifestSummary(
  data: ReleaseData,
  channelId: string,
  manifestRecord: JsonObject,
): { coverageLabel: string } {
  const manifest =
    data.sourceMode === 'graph'
      ? data.graph?.manifests?.[channelId]?.[manifestRecord.version]
      : channelId === data.channel
        ? data.manifest
        : undefined;
  if (!manifest) {
    return { coverageLabel: 'not published' };
  }
  const packages = Array.isArray(manifest.packages) ? manifest.packages : [];
  const profiles = Object.values(manifest.profiles ?? {}) as JsonObject[];
  const architectures = Array.from(
    new Set(
      profiles.flatMap((profile) => profileArchNames(profile)),
    ),
  ).sort();
  const archLabel = architectures.length > 0 ? architectures.join(', ') : 'no architectures';
  return {
    coverageLabel: `${packages.length} packages / ${profiles.length} profiles / ${archLabel}`,
  };
}

export function dataForChannel(data: ReleaseData, channel: string): ReleaseData {
  const channelRecord = data.channels.channels?.[channel];
  if (!channelRecord) {
    throw new Error(`Unknown release channel: ${channel}`);
  }
  const manifestRecord = selectManifestRecord(channelRecord);
  if (data.sourceMode === 'graph') {
    const manifest = data.graph?.manifests?.[channel]?.[manifestRecord.version];
    if (!manifest) {
      throw new Error(`Release graph is missing ${channel} manifest ${manifestRecord.version}`);
    }
    const profileContract = {
      schema: 'capsem.manifest_profiles.v1',
      revision: profileRevisionFromManifest(manifest),
      profiles: profileListFromManifest(manifest),
    };
    return {
      ...data,
      channel,
      channelRecord,
      manifestRecord,
      manifest,
      profileContract,
    };
  }

  const manifestPath = trimLeadingSlash(String(manifestRecord.url ?? `/assets/${channel}/manifest.json`));
  const manifest = readJson(resolve(data.dist, manifestPath));
  const profileContract = profileContractFromManifest(manifest);
  return {
    ...data,
    channel,
    channelRecord,
    manifestRecord,
    manifest,
    profileContract,
  };
}

export function profileList(data: ReleaseData): JsonObject[] {
  const profiles = Array.isArray(data.profileContract.profiles)
    ? data.profileContract.profiles
    : profileListFromManifest(data.manifest);
  return profiles.map((profile) => normalizeProfile(profile));
}

export function profileById(data: ReleaseData, id: string): JsonObject | undefined {
  return profileList(data).find((profile) => profile.id === id);
}

export function profileArchNames(profile: JsonObject): string[] {
  if (Array.isArray(profile.architectures)) {
    return profile.architectures
      .map((architecture: JsonObject) => String(architecture.architecture ?? ''))
      .filter(Boolean)
      .sort();
  }
  const legacy = Object.keys(profile.assets?.arch ?? {});
  const graph = Array.isArray(profile.images)
    ? profile.images.map((image: JsonObject) => String(image.architecture ?? '')).filter(Boolean)
    : profile.images && typeof profile.images === 'object'
      ? Object.keys(profile.images)
    : [];
  return Array.from(new Set([...legacy, ...graph])).sort();
}

export function profileArchitectures(profile: JsonObject): JsonObject[] {
  if (Array.isArray(profile.architectures)) {
    return profile.architectures.map((architecture: JsonObject) => ({
      architecture: architecture.architecture,
      software: Array.isArray(architecture.software) ? architecture.software : [],
      config: Array.isArray(architecture.config) ? architecture.config : [],
      images: Array.isArray(architecture.images) ? architecture.images : [],
      evidence: Array.isArray(architecture.evidence) ? architecture.evidence : [],
    }));
  }
  const imageRecords = Array.isArray(profile.images)
    ? profile.images
    : Object.entries(profile.images ?? {}).map(([architecture, value]) => ({
        architecture,
        ...(value as Record<string, unknown>),
      }));
  return imageRecords.map((image: JsonObject) => ({
    architecture: image.architecture,
    software: Array.isArray(profile.software)
      ? profile.software.filter((item: JsonObject) => {
          const itemArch = String(item.architecture ?? '');
          return itemArch === image.architecture || itemArch === 'all';
        })
      : [],
    config: Array.isArray(profile.config) ? profile.config : profileFileRows(profile),
    images: Array.isArray(image.artifacts) ? image.artifacts : [],
    evidence: Array.isArray(image.evidence) ? image.evidence : [],
  }));
}

export function profileArtifactRows(profile: JsonObject, arch: string): TableRow[] {
  if (profile.images && typeof profile.images === 'object' && !Array.isArray(profile.images)) {
    const imageSet = profile.images[arch] ?? {};
    const artifacts = Array.isArray(imageSet.artifacts) ? imageSet.artifacts : [];
    const evidence = Array.isArray(imageSet.evidence) ? imageSet.evidence : [];
    return [
      ...artifacts.map((artifact: JsonObject) => descriptorRow(artifactLabel(artifact.kind), artifact)),
      ...evidence.map((item: JsonObject) => descriptorRow(evidenceLabel(item.kind), item)),
    ];
  }
  if (Array.isArray(profile.images)) {
    const imageSet = profile.images.find((image: JsonObject) => image.architecture === arch) ?? {};
    const artifacts = Array.isArray(imageSet.artifacts) ? imageSet.artifacts : [];
    const evidence = Array.isArray(imageSet.evidence) ? imageSet.evidence : [];
    return [
      ...artifacts.map((artifact: JsonObject) => descriptorRow(artifactLabel(artifact.kind), artifact)),
      ...evidence.map((item: JsonObject) => descriptorRow(evidenceLabel(item.kind), item)),
    ];
  }

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
    rows.push({ label: 'ABOM / OBOM', name: 'Not published in profile evidence', status: 'missing' });
  }

  const sbom = profile.sbom?.arch?.[arch];
  if (sbom) {
    rows.push(descriptorRow('SBOM', sbom));
  } else {
    rows.push({ label: 'SBOM', name: 'Not published in profile evidence', status: 'missing' });
  }
  return rows;
}

export function profileFileRows(profile: JsonObject): TableRow[] {
  if (Array.isArray(profile.config)) {
    return profile.config.map((item: JsonObject) => ({
      label: String(item.kind ?? 'config'),
      name: String(item.path ?? item.url ?? ''),
      url: item.url,
      size: item.bytes ?? item.size,
      hash: item.digest?.blake3 ?? item.hash,
      status: item.status,
    }));
  }
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
  if (Array.isArray(data.manifest.packages)) {
    return data.manifest.packages.flatMap((pkg: JsonObject) => {
      const evidence = Array.isArray(pkg.evidence) ? pkg.evidence : [];
      return Array.isArray(pkg.binaries)
        ? pkg.binaries.map((binary: JsonObject) => ({
            ...binary,
            package_name: pkg.name,
            package_id: pkg.id,
            package_evidence: evidence,
          }))
        : [];
    });
  }
  return [];
}

export function packageRows(data: ReleaseData): JsonObject[] {
  return Array.isArray(data.manifest.packages) ? data.manifest.packages : [];
}

export function packageTargetLabel(pkg: JsonObject): string {
  const architecture = String(pkg.architecture ?? 'unknown');
  const platform = String(pkg.platform ?? 'unknown');
  const platformLabel = platform === 'macos'
    ? 'macOS'
    : platform.charAt(0).toUpperCase() + platform.slice(1);
  return `${platformLabel} ${architecture}`;
}

export function packageById(data: ReleaseData, id: string): JsonObject | undefined {
  return packageRows(data).find((pkg) => String(pkg.id) === id);
}

export function manifestRecords(data: ReleaseData): JsonObject[] {
  return Array.isArray(data.channelRecord.manifests) ? data.channelRecord.manifests : [];
}

export function hostSbomRows(data: ReleaseData): JsonObject[] {
  if (Array.isArray(data.manifest.packages)) {
    return data.manifest.packages.flatMap((pkg: JsonObject) => {
      const evidence = Array.isArray(pkg.evidence) ? pkg.evidence : [];
      return evidence.filter((item: JsonObject) => String(item.kind ?? '').toLowerCase().includes('sbom'));
    });
  }
  return [];
}

function profileEvidenceRows(data: ReleaseData): JsonObject[] {
  return profileList(data)
    .flatMap((profile): JsonObject[] => {
      if (profile.images && typeof profile.images === 'object' && !Array.isArray(profile.images)) {
        return Object.entries(profile.images).flatMap(([arch, imageSet]) => {
          const image = imageSet as JsonObject;
          const evidence: JsonObject[] = Array.isArray(image.evidence) ? image.evidence : [];
          return evidence.map((item: JsonObject) => ({
            profile: profile.id,
            arch,
            logical_name: item.kind,
            ...item,
          }));
        });
      }
      if (Array.isArray(profile.images)) {
        return (profile.images as JsonObject[]).flatMap((image: JsonObject): JsonObject[] => {
          const evidence: JsonObject[] = Array.isArray(image.evidence) ? image.evidence : [];
          return evidence
            .filter((item: JsonObject) => ['abom', 'obom', 'sbom'].includes(String(item.kind ?? '').toLowerCase()))
            .map((item: JsonObject) => ({
              profile: profile.id,
              arch: image.architecture,
              logical_name: item.kind,
              ...item,
            }));
        });
      }
      const obomByArch = profile.obom?.arch ?? {};
      return Object.entries(obomByArch).map(([arch, descriptor]) => ({
        arch,
        ...(descriptor as JsonObject),
      }));
    })
    .sort((left, right) => String(left.arch).localeCompare(String(right.arch)));
}

function currentProfileFilesByArch(data: ReleaseData): [string, JsonObject[]][] {
  const graphFiles = profileList(data).flatMap((profile) => {
    if (profile.images && typeof profile.images === 'object' && !Array.isArray(profile.images)) {
      return Object.entries(profile.images).flatMap(([arch, imageSet]) => {
        const image = imageSet as JsonObject;
        const artifacts = Array.isArray(image.artifacts) ? image.artifacts : [];
        return artifacts.map((artifact: JsonObject) => ({
          arch,
          logical_name: `${profile.id}/${artifact.name ?? artifact.kind}`,
          ...artifact,
        }));
      });
    }
    if (!Array.isArray(profile.images)) return [];
    return profile.images.flatMap((image: JsonObject) => {
      const artifacts = Array.isArray(image.artifacts) ? image.artifacts : [];
      return artifacts.map((artifact: JsonObject) => ({
        arch: image.architecture,
        logical_name: `${profile.id}/${artifact.name ?? artifact.kind}`,
        ...artifact,
      }));
    });
  });
  if (graphFiles.length > 0) {
    return groupFilesByArch(graphFiles);
  }

  const current = String(data.manifest.assets?.current ?? '');
  const release = data.manifest.assets?.releases?.[current] ?? {};
  const arches = release.arches ?? {};
  const base = currentProfileBaseUrl(data);
  const files = Object.entries(arches).flatMap(([arch, entries]) => {
    return Object.entries(entries as JsonObject).map(([logicalName, entry]) => ({
      arch,
      logical_name: logicalName,
      url: assetFileUrl(base, arch, logicalName),
      ...(entry as JsonObject),
    }));
  });
  return groupFilesByArch(files);
}

function releaseHistoryRows(data: ReleaseData): JsonObject[] {
  if (Array.isArray(data.channelRecord.manifests) && !data.manifest.assets?.releases) {
    return data.channelRecord.manifests.map((manifest: JsonObject) => ({
      version: manifest.version,
      date: manifest.date ?? '',
      state: manifest.status,
      deprecated: manifest.status === 'deprecated',
      deprecated_date: manifest.deprecated_date,
      min_binary: manifest.min_capsem_version,
      arches: currentArchitectures(data),
    }));
  }
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
  const graphArches = profileList(data).flatMap((profile) => profileArchNames(profile));
  if (graphArches.length > 0) {
    return Array.from(new Set(graphArches)).sort();
  }
  return Object.keys(
    data.manifest.assets?.releases?.[data.manifest.assets?.current]?.arches ?? {},
  ).sort();
}

function currentProfileBaseUrl(data: ReleaseData): string {
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

export function profileRevision(data: ReleaseData): string {
  return String(data.profileContract.revision ?? '');
}

export function manifestUrl(data: ReleaseData): string {
  return String(data.manifestRecord.url ?? `/assets/${data.channel}/manifest.json`);
}

export function manifestBlake3(data: ReleaseData): string {
  return hashLabel(data.manifestRecord.digest?.blake3);
}

export function byteLabel(value: unknown): string {
  return typeof value === 'number' ? value.toLocaleString('en-US') : 'unknown';
}

export function hashLabel(value: unknown): string {
  if (typeof value !== 'string' || value.length === 0) {
    return 'not published';
  }
  return value.length > 12 ? `${value.slice(0, 8)}...` : value;
}

function descriptorRow(label: string, descriptor: JsonObject): TableRow {
  return {
    label,
    name: String(descriptor.name ?? descriptor.kind ?? ''),
    url: descriptor.url,
    size: descriptor.bytes ?? descriptor.size,
    hash: descriptor.digest?.blake3 ?? descriptor.hash,
    status: descriptor.status,
  };
}

function groupFilesByArch(files: JsonObject[]): [string, JsonObject[]][] {
  const grouped = new Map<string, JsonObject[]>();
  for (const file of files) {
    const arch = String(file.arch ?? file.architecture ?? 'unknown');
    const rows = grouped.get(arch) ?? [];
    rows.push(file);
    grouped.set(arch, rows);
  }
  return Array.from(grouped.entries())
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([arch, rows]) => [
      arch,
      rows.sort((left, right) => String(left.logical_name ?? left.name ?? '').localeCompare(String(right.logical_name ?? right.name ?? ''))),
    ]);
}

function profileListFromManifest(manifest: JsonObject): JsonObject[] {
  return Object.entries(manifest.profiles ?? {}).map(([id, profile]) => normalizeProfile({ id, ...(profile as JsonObject) }));
}

function profileContractFromManifest(manifest: JsonObject): JsonObject {
  return {
    schema: 'capsem.manifest_profiles.v1',
    revision: profileRevisionFromManifest(manifest),
    profiles: profileListFromManifest(manifest),
  };
}

function profileRevisionFromManifest(manifest: JsonObject): string {
  return profileListFromManifest(manifest).map((profile) => profile.revision).filter(Boolean).join(', ');
}

function normalizeProfile(profile: JsonObject): JsonObject {
  const id = String(profile.id ?? 'unknown');
  const name = profile.name ?? id.split('-').map((part) => part.charAt(0).toUpperCase() + part.slice(1)).join(' ');
  return {
    ...profile,
    id,
    name,
    description: profile.description ?? `Release profile ${id}`,
  };
}

function artifactLabel(kind: unknown): string {
  switch (String(kind ?? '').toLowerCase()) {
    case 'kernel':
      return 'Kernel';
    case 'initrd':
      return 'Initrd';
    case 'rootfs':
      return 'Root filesystem';
    default:
      return 'Profile artifact';
  }
}

function evidenceLabel(kind: unknown): string {
  const raw = String(kind ?? 'evidence').toUpperCase();
  return raw === 'OBOM' ? 'OBOM' : raw;
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

function assetFileUrl(baseUrl: string, arch: string, logicalName: string): string {
  const normalizedBase = baseUrl.replace(/\/+$/, '');
  return `${normalizedBase}/${arch}-${logicalName}`;
}

function readJson(path: string): JsonObject {
  if (!existsSync(path)) {
    throw new Error(`Release-site input is missing: ${path}`);
  }
  return JSON.parse(readFileSync(path, 'utf8')) as JsonObject;
}

function resolveReleaseInput(path: string): string {
  if (isAbsolute(path)) {
    return path;
  }
  const fromCwd = resolve(process.cwd(), path);
  if (existsSync(fromCwd)) {
    return fromCwd;
  }
  return resolve(process.cwd(), '..', path);
}

function isJsonFile(path: string): boolean {
  return existsSync(path) && statSync(path).isFile() && path.endsWith('.json');
}

function trimLeadingSlash(path: string): string {
  return path.replace(/^\/+/, '');
}

function isHostSbom(name: unknown): boolean {
  return name === 'capsem-sbom.spdx.json';
}
