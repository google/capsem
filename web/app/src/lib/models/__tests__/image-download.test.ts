import { expect, it } from 'vitest';
import { ImageDownloadPhase } from '@capsem/sdk';
import { imageDownloadView } from '../image-download';

it('keeps preparation indeterminate without inventing bytes or a percentage', () => {
  expect(imageDownloadView()).toMatchObject({ phase: 'Preparing', label: 'Preparing', percent: null });
});

it('shows real byte counts while the complete transfer size remains unknown', () => {
  const view = imageDownloadView({ phase: ImageDownloadPhase.DOWNLOADING, completed_bytes: 2 * 1024 * 1024 });
  expect(view).toMatchObject({ phase: 'Downloading', label: '2.0 MB', percent: null });
});

it('maps declared byte totals to an accessible bounded percentage', () => {
  for (const percent of [0, 25, 50, 99]) {
    const view = imageDownloadView({ phase: ImageDownloadPhase.DOWNLOADING, completed_bytes: percent * 1024, total_bytes: 100 * 1024 });
    expect(view.percent).toBe(percent);
    expect(view.label).toContain(`${percent}%`);
    expect(view.valueText).toContain('100.0 KB');
  }
});

it('keeps verification visible after all bytes arrive', () => {
  const view = imageDownloadView({ phase: ImageDownloadPhase.VERIFYING, completed_bytes: 1024, total_bytes: 1024 });
  expect(view).toMatchObject({ phase: 'Verifying', label: 'Verifying', percent: 100 });
  expect(view.valueText).toContain('Verifying');
});

it('never turns absent, zero or inconsistent totals into a fake full bar', () => {
  for (const total_bytes of [undefined, null, 0, 10]) {
    expect(imageDownloadView({ phase: ImageDownloadPhase.DOWNLOADING, completed_bytes: 20, total_bytes }).percent).toBeNull();
  }
});
