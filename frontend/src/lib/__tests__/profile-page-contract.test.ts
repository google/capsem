import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const source = readFileSync(
  new URL('../components/shell/ProfilePage.svelte', import.meta.url),
  'utf8',
);

describe('ProfilePage route contract', () => {
  it('exposes enforcement and detection as first-class tabs, not a generic policy tab', () => {
    expect(source).toContain("key: 'enforcement'");
    expect(source).toContain("key: 'detection'");
    expect(source).not.toContain("key: 'policy'");
    expect(source).not.toContain("label: 'Policy'");
  });

  it('renders profile asset status from the typed status route instead of raw JSON', () => {
    expect(source).toContain('getAssetsStatus');
    expect(source).toContain('assetStatusLabel');
    expect(source).not.toContain('getProfileAssetsInfo');
    expect(source).not.toContain('JSON.stringify(assetsInfo');
  });
});
