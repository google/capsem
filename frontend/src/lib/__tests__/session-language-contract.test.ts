import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const dashboard = readFileSync(
  new URL('../components/shell/NewTabPage.svelte', import.meta.url),
  'utf8',
);
const appShell = readFileSync(
  new URL('../components/shell/App.svelte', import.meta.url),
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
const legacyVmSingular = 'V' + 'M';
const legacyVmPlural = legacyVmSingular + 's';

describe('user-facing session language contract', () => {
  it('uses sessions on the dashboard instead of VM wording', () => {
    expect(dashboard).toContain('Sessions');
    expect(dashboard).toContain('Loading sessions');
    expect(dashboard).toContain('No sessions');
    expect(dashboard).toContain('Failed to create session');
    expect(dashboard).not.toContain('>' + legacyVmPlural + '<');
    expect(dashboard).not.toContain('Customize ' + 'VM');
    expect(dashboard).not.toContain('Loading ' + legacyVmPlural);
    expect(dashboard).not.toContain('No ' + legacyVmPlural);
    expect(dashboard).not.toContain('Failed to create VM');
  });

  it('keeps profile creation controls on each profile card', () => {
    expect(dashboard).toContain('New');
    expect(dashboard).toContain('Customize');
    expect(dashboard).toContain('openCustomizeProfile');
    expect(dashboard).toContain('profileAssetChecklist');
    expect(dashboard).toContain('VM assets');
    expect(dashboard).toContain('profileAssetText(launcher.assets)');
    expect(dashboard).toContain('launcher.assets?.ready === true');
    expect(dashboard).toContain("onclick={() => ready ? createFromProfile(launcher.profile.id) : ensureProfileAssets(launcher.profile.id)}");
    expect(dashboard).toContain("title={ready ? `New ${launcher.profile.name} session` : profileAssetText(launcher.assets)}");
    expect(dashboard).toContain("asset.status === 'present'");
    expect(dashboard).toContain("asset.status === 'downloading'");
    expect(dashboard).toContain('<CheckCircle');
    expect(dashboard).toContain('<DownloadSimple');
    expect(dashboard).not.toContain('Customize Session...');
    expect(dashboard).not.toContain('vmStore.showCreateModal = true');
  });

  it('does not duplicate profile actions in the card header and footer', () => {
    expect(dashboard).toContain("title={ready ? `New ${launcher.profile.name} session` : profileAssetText(launcher.assets)}");
    expect(dashboard).not.toContain('aria-label={profileAssetText(launcher.assets)}');
    expect(dashboard).not.toContain('Start</span>');
  });

  it('lets the service own quick-create session names and profile resources', () => {
    const quickCreateSources = [dashboard, appShell].join('\n');
    expect(quickCreateSources).not.toContain('generatedVmName');
    expect(quickCreateSources).not.toContain('name: generatedVmName');
    expect(quickCreateSources).not.toContain('ram_mb: 2048');
    expect(quickCreateSources).not.toContain('cpus: 2');
    expect(dashboard).toContain('profile_id: profileId');
    expect(appShell).toContain("profile_id: 'code'");
    expect(quickCreateSources).toContain('persistent: true');
  });

  it('uses sessions in toolbar controls and keeps build stamp out of visible chrome', () => {
    expect(toolbar).toContain('Session Logs');
    expect(toolbar).toContain('session');
    expect(toolbar).not.toContain('Frontend build');
    expect(toolbar).not.toContain('build {__BUILD_TS__}');
    expect(toolbar).not.toContain('VM Logs');
  });

  it('uses semantic tokens for toolbar status chrome', () => {
    expect(toolbar).toContain("'bg-primary'");
    expect(toolbar).toContain("'bg-warning'");
    expect(toolbar).toContain("'bg-destructive'");
    expect(toolbar).not.toContain('bg-green-');
    expect(toolbar).not.toContain('bg-amber-');
    expect(toolbar).not.toContain('bg-red-');
  });

  it('uses session wording in stats subtitles', () => {
    expect(stats).toContain('Session {vmId} ledger');
    expect(stats).not.toContain('Session {vmId} database');
    expect(stats).not.toContain('VM {vmId} session database');
  });
});
