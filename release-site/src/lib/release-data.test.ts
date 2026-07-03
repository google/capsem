import { beforeEach, describe, expect, it } from 'vitest';

import {
  channelRows,
  dataForChannel,
  hashLabel,
  loadReleaseData,
  packageRows,
  profileList,
} from './release-data';

describe('release-site graph data', () => {
  beforeEach(() => {
    process.env.CAPSEM_RELEASE_CHANNEL_DIST =
      '../tests/capsem-release/fixtures/release-graph-stable-nightly.json';
    process.env.CAPSEM_RELEASE_CHANNEL = 'stable';
  });

  it('loads channel rows from the generated release graph', () => {
    const data = loadReleaseData();
    const rows = channelRows(data);

    expect(data.sourceMode).toBe('graph');
    expect(rows.map((row) => row.id)).toEqual(['nightly', 'stable']);
    expect(rows.find((row) => row.id === 'stable')).toMatchObject({
      description: 'Recommended release channel for everyday Capsem installs.',
      manifestUrl: '/assets/stable/manifest.json',
    });
  });

  it('selects package and profile data for a channel', () => {
    const data = dataForChannel(loadReleaseData(), 'stable');

    expect(packageRows(data).map((pkg) => pkg.id)).toContain('capsem-1-4-0-pkg');
    expect(profileList(data).map((profile) => profile.id).sort()).toEqual([
      'co-work',
      'code',
    ]);
  });

  it('keeps human digest display short without changing source data', () => {
    expect(hashLabel('1234567890abcdef')).toBe('12345678...');
    expect(hashLabel('12345678')).toBe('12345678');
    expect(hashLabel(undefined)).toBe('not published');
  });
});
