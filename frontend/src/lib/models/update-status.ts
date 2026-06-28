import type { SupplyChainReference, UpdateStatusResponse, UpdateTrackStatus } from '../types/gateway';

export type UpdateTrackKey = 'binary' | 'assets' | 'profiles' | 'images';
export type UpdateTrackTone = 'available' | 'blocked' | 'muted';

export interface UpdateEvidenceLink {
  label: string;
  href: string;
  meta?: string;
}

export const UPDATE_TRACK_LABELS: Record<UpdateTrackKey, string> = {
  binary: 'Binary',
  assets: 'VM assets',
  profiles: 'Profiles',
  images: 'VM images',
};

export function updateAvailableTracks(status: UpdateStatusResponse): UpdateTrackKey[] {
  const tracks: UpdateTrackKey[] = [];
  if (status.binary.update_available) tracks.push('binary');
  if (status.assets.update_available) tracks.push('assets');
  if (status.profiles.update_available) tracks.push('profiles');
  if (status.images.update_available) tracks.push('images');
  return tracks;
}

export function updateSummary(status: UpdateStatusResponse): string {
  const tracks = updateAvailableTracks(status);
  if (tracks.length > 0) {
    return `${tracks.map(track => UPDATE_TRACK_LABELS[track]).join(', ')} available`;
  }
  if (status.last_error) return 'Update status unavailable';
  if (status.stale) return 'Update check stale';
  return 'Up to date';
}

export function updateTrackVersion(track: UpdateTrackStatus): string {
  const current = track.current ?? 'unknown';
  const latest = track.latest ?? current;
  if (track.update_available) return `${current} -> ${latest}`;
  if (track.state === 'not_published') return 'not published';
  return current;
}

export function updateTrackDetail(track: UpdateTrackStatus): string | null {
  if (track.blocked_reason) return track.blocked_reason;
  if (track.compatibility === 'unknown') return 'Compatibility unknown';
  if (track.compatibility === 'not_applicable') return null;
  return null;
}

export function updateTrackStateLabel(track: UpdateTrackStatus): string {
  if (track.blocked_reason) return 'Blocked';
  if (track.update_available) return 'Update available';
  if (track.state === 'not_published') return 'Not published';
  if (track.state === 'unknown') return 'Unknown';
  return 'Current';
}

export function updateTrackTone(track: UpdateTrackStatus): UpdateTrackTone {
  if (track.blocked_reason) return 'blocked';
  if (track.update_available) return 'available';
  return 'muted';
}

export function updateEvidenceLinks(status: UpdateStatusResponse): UpdateEvidenceLink[] {
  const evidence = status.supply_chain;
  if (!evidence) return [];

  const links: UpdateEvidenceLink[] = [];
  const host = referenceLink('Host SBOM', evidence.host_sbom);
  if (host) links.push(host);
  const vm = referenceLink('VM OBOM', evidence.vm_obom);
  if (vm) links.push(vm);

  for (const attestation of evidence.attestations ?? []) {
    const label = attestation.scope === 'vm_assets'
      ? 'VM asset attestation'
      : attestation.scope === 'binary'
        ? 'Binary attestation'
        : attestation.name || 'Attestation';
    const link = referenceLink(label, attestation);
    if (link) links.push(link);
  }

  return links;
}

function referenceLink(label: string, reference: SupplyChainReference | undefined): UpdateEvidenceLink | null {
  if (!reference) return null;
  const href = reference.route || reference.release_artifact;
  if (!href) return null;
  const meta = [reference.format, reference.scope].filter(Boolean).join(' · ');
  return {
    label,
    href,
    meta: meta || undefined,
  };
}
