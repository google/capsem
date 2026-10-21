import { ImageDownloadPhase, type ImageDownloadProgress } from '@capsem/sdk';
import { formatBytes } from '../format';

/** A bar describes observed work; an unknown total never becomes a percentage. */
export function imageDownloadView(progress?: ImageDownloadProgress | null) {
  const phase = progress?.phase === ImageDownloadPhase.VERIFYING ? 'Verifying'
    : progress?.phase === ImageDownloadPhase.DOWNLOADING ? 'Downloading' : 'Preparing';
  const completed = Math.max(0, progress?.completed_bytes ?? 0);
  const total = progress?.total_bytes;
  const percent = total && total >= completed ? Math.floor(completed / total * 100) : null;
  const bytes = formatBytes(completed);
  const label = phase !== 'Downloading' ? phase : percent === null ? bytes : `${percent}% · ${bytes}`;
  const valueText = total && percent !== null ? `${phase}, ${bytes} of ${formatBytes(total)}, ${percent}%` : `${phase}, ${bytes}`;
  return { phase, label, percent, valueText };
}
