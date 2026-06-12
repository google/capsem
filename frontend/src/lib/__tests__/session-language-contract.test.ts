import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const dashboard = readFileSync(
  new URL('../components/shell/NewTabPage.svelte', import.meta.url),
  'utf8',
);
const toolbar = readFileSync(
  new URL('../components/shell/Toolbar.svelte', import.meta.url),
  'utf8',
);
const stats = readFileSync(
  new URL('../components/views/StatsView.svelte', import.meta.url),
  'utf8',
);

describe('user-facing session language contract', () => {
  it('uses sessions on the dashboard instead of VM wording', () => {
    expect(dashboard).toContain('Sessions');
    expect(dashboard).toContain('Loading sessions');
    expect(dashboard).toContain('No sessions');
    expect(dashboard).toContain('Failed to create session');
    expect(dashboard).not.toContain('>VMs<');
    expect(dashboard).not.toContain('Customize VM');
    expect(dashboard).not.toContain('Loading VMs');
    expect(dashboard).not.toContain('No VMs');
    expect(dashboard).not.toContain('Failed to create VM');
  });

  it('uses sessions in toolbar controls and hides build stamp on session tabs', () => {
    expect(toolbar).toContain('Session Logs');
    expect(toolbar).toContain('session');
    expect(toolbar).toContain('{#if !isVM}');
    expect(toolbar).not.toContain('VM Logs');
  });

  it('uses session wording in stats subtitles', () => {
    expect(stats).toContain('Session {vmId} database');
    expect(stats).not.toContain('VM {vmId} session database');
  });
});
