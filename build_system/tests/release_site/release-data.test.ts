import { beforeEach, describe, expect, it } from 'vitest';

import {
  channelRows,
  dataForChannel,
  hashLabel,
  loadReleaseData,
  packageRows,
  runtimeArchNames,
  runtimeRecord,
} from '../../release_site/src/lib/release-data';

describe('release-site graph data', () => {
  beforeEach(() => {
    process.env.CAPSEM_RELEASE_GRAPH =
      '../../tests/capsem-release/fixtures/release-graph-stable-nightly.json';
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

  it('selects package and runtime data for a channel', () => {
    const data = dataForChannel(loadReleaseData(), 'stable');
    const runtime = runtimeRecord(data);

    expect(packageRows(data).map((pkg) => pkg.id)).toContain('capsem-1-4-0-pkg');
    expect(runtime?.revision).toBe('1.0.0-stable.20260702');
    expect(runtimeArchNames(runtime ?? {})).toEqual(['arm64', 'x86_64']);
  });

  it('treats a manifest without a runtime as unpublished', () => {
    const data = dataForChannel(loadReleaseData(), 'stable');

    expect(runtimeRecord({ ...data, manifest: { ...data.manifest, runtime: null } })).toBeUndefined();
  });

  it('keeps human digest display short without changing source data', () => {
    expect(hashLabel('1234567890abcdef')).toBe('12345678...');
    expect(hashLabel('12345678')).toBe('12345678');
    expect(hashLabel(undefined)).toBe('not published');
  });
});
