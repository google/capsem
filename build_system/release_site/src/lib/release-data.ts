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
}

export interface ChannelRow {
  id: string;
  label: string;
  description: string;
  manifestCount: number;
  manifestRevision: string;
  binaryLabel: string;
  assetLabel: string;
  currentStatus: string;
  statuses: string[];
  updatedAt: string;
  coverageLabel: string;
  manifestUrl: string;
  pageUrl: string;
}

export function loadReleaseData(): ReleaseData {
  // The graph to render. Named for what it is: this used to read
  // CAPSEM_RELEASE_CHANNEL_DIST, which the overlay also read to decide where to
  // *write* -- so one name selected an input in one place and an output in
  // another, and nothing but convention kept a caller from crossing them.
  const graphEnv = process.env.CAPSEM_RELEASE_GRAPH ?? DEFAULT_RELEASE_GRAPH;
  const dist = resolveReleaseInput(graphEnv);
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
  return { dist, sourceMode: 'dist', channel, channels, channelRecord, manifestRecord, manifest };
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
  return {
    dist: graphPath,
    sourceMode: 'graph',
    graph,
    channel,
    channels,
    channelRecord,
    manifestRecord,
    manifest,
  };
}

export function channelRuntimePagePath(channelId: string): string {
  return `/channels/${encodeURIComponent(channelId)}/runtime/`;
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
        binaryLabel: summary.binaryLabel,
        assetLabel: summary.assetLabel,
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
): { coverageLabel: string; binaryLabel: string; assetLabel: string } {
  const manifest =
    data.sourceMode === 'graph'
      ? data.graph?.manifests?.[channelId]?.[manifestRecord.version]
      : channelId === data.channel
        ? data.manifest
        : undefined;
  if (!manifest) {
    return { coverageLabel: 'not published', binaryLabel: 'not published', assetLabel: 'not published' };
  }
  const packages = Array.isArray(manifest.packages) ? manifest.packages : [];
  const runtime = runtimeFromManifest(manifest);
  const architectures = runtime ? runtimeArchNames(runtime) : [];
  const archLabel = architectures.length > 0 ? architectures.join(', ') : 'no architectures';
  const binaryLabel = packages.length > 0 ? String(packages[0].version ?? 'not published') : 'not published';
  const assetLabel = runtime ? String(runtime.revision ?? 'not published') : 'not published';
  return {
    binaryLabel,
    assetLabel,
    coverageLabel: `${packages.length} packages / runtime ${assetLabel} / ${archLabel}`,
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
    return { ...data, channel, channelRecord, manifestRecord, manifest };
  }

  const manifestPath = trimLeadingSlash(String(manifestRecord.url ?? `/assets/${channel}/manifest.json`));
  const manifest = readJson(resolve(data.dist, manifestPath));
  return { ...data, channel, channelRecord, manifestRecord, manifest };
}

/** The channel's one runtime, or undefined when the channel has published none yet. */
export function runtimeRecord(data: ReleaseData): JsonObject | undefined {
  return runtimeFromManifest(data.manifest);
}

export function runtimeArchNames(runtime: JsonObject): string[] {
  return runtimeArchitectures(runtime)
    .map((architecture) => String(architecture.architecture ?? ''))
    .filter(Boolean)
    .sort();
}

export function runtimeArchitectures(runtime: JsonObject): JsonObject[] {
  const architectures = Array.isArray(runtime.architectures) ? runtime.architectures : [];
  return architectures.map((architecture: JsonObject) => ({
    architecture: architecture.architecture,
    image_revision: architecture.image_revision,
    package_inventory_revision: architecture.package_inventory_revision,
    software: Array.isArray(architecture.software) ? architecture.software : [],
    images: Array.isArray(architecture.images) ? architecture.images : [],
    evidence: Array.isArray(architecture.evidence) ? architecture.evidence : [],
  }));
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

export function generatedAt(data: ReleaseData): string {
  return String(data.channels.generated_at ?? '');
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

function runtimeFromManifest(manifest: JsonObject): JsonObject | undefined {
  const runtime = manifest.runtime;
  return runtime && typeof runtime === 'object' && !Array.isArray(runtime) ? runtime : undefined;
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
