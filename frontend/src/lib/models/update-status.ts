import type { UpdateStatusResponse, UpdateTrackStatus } from '../types/gateway';

export type UpdateTrackKey = 'binary' | 'assets' | 'profiles' | 'images';

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
