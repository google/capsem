import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { sessionActionError } from '../models/session-action-error';

const dashboard = readFileSync(
  new URL('../components/shell/NewTabPage.svelte', import.meta.url),
  'utf8',
);
const appShell = readFileSync(
  new URL('../components/shell/App.svelte', import.meta.url),
  'utf8',
);
const createDialog = readFileSync(
  new URL('../components/shell/CreateSandboxDialog.svelte', import.meta.url),
  'utf8',
);
const toolbar = readFileSync(
  new URL('../components/shell/Toolbar.svelte', import.meta.url),
  'utf8',
);
const settings = readFileSync(
  new URL('../components/shell/SettingsPage.svelte', import.meta.url),
  'utf8',
);
const about = readFileSync(
  new URL('../components/shell/AboutPage.svelte', import.meta.url),
  'utf8',
);
const app = readFileSync(
  new URL('../components/shell/App.svelte', import.meta.url),
  'utf8',
);
const vmStore = readFileSync(
  new URL('../stores/vms.svelte.ts', import.meta.url),
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
    expect(dashboard).toContain('actionError = sessionActionError(error)');
    expect(sessionActionError(null)).toBe('Failed to create session');
    expect(dashboard).not.toContain('>' + legacyVmPlural + '<');
    expect(dashboard).not.toContain('Customize ' + 'VM');
    expect(dashboard).not.toContain('Loading ' + legacyVmPlural);
    expect(dashboard).not.toContain('No ' + legacyVmPlural);
    expect(dashboard).not.toContain('Failed to create VM');
  });

  it('keeps one new-session launcher gated on VM asset readiness', () => {
    expect(dashboard).toContain('New session');
    expect(dashboard).toContain('Customize');
    expect(dashboard).toContain('vmStore.openCreateModal()');
    expect(dashboard).toContain('api.getAssetsStatus()');
    expect(dashboard).toContain('api.ensureAssets()');
    expect(dashboard).toContain('let ready = $derived(assets?.ready === true)');
    expect(dashboard).toContain('onclick={() => ready ? createSession() : downloadAssets()}');
    expect(dashboard).toContain("title={ready ? 'New session' : assetText(assets)}");
    expect(dashboard).toContain('<DownloadSimple');
    expect(dashboard).toContain('Downloading');
    expect(dashboard).toContain('role="progressbar"');
    expect(dashboard).toContain('aria-valuenow={assetPercent(assets)}');
    expect(dashboard).not.toContain('Customize Session...');
    expect(dashboard).not.toContain('vmStore.showCreateModal = true');
    expect(dashboard).not.toContain('getUpdateStatus');
    expect(dashboard).not.toContain('Not published');
    expect(dashboard).not.toContain('Start</span>');
  });

  it('routes About Capsem to a top-level canonical status page', () => {
    expect(toolbar).toContain("tabStore.openSingleton('about', 'About Capsem')");
    expect(app).toContain("tab.view === 'about'");
    expect(app).toContain('loadAbout()');
    expect(settings).not.toContain('About Capsem');
    expect(settings).not.toContain('Release diagnostics');
    expect(settings).not.toContain('debugSnapshot');
    expect(about).toContain('About Capsem');
    expect(about).toContain('api.getCapsemStatus()');
    expect(about).toContain('api.checkForUpdates()');
    expect(about).toContain('system?.manifest.packages');
    expect(about).toContain('system.manifest_metadata.manifest_url');
    expect(about).toContain('system.assets.assets');
    expect(about).toContain('system?.assets.current_arch');
    expect(about).toContain('packageEvidence(pkg)');
    expect(about).toContain('evidence.url');
    expect(about).toContain('<details');
    expect(about).toContain('Installed package binaries');
    expect(about).toContain('Channel package differs from installed Capsem');
    expect(about).toContain('packages.filter((pkg) => pkg.platform === platform)');
    expect(about).not.toContain('The installed Capsem version is absent from the installed release manifest.');
    expect(about).not.toContain('updates.supply_chain');
    expect(about).not.toContain('Not published');
    expect(about).not.toContain("trackLabel(key: UpdateTrackKey): string");
    expect(about).not.toContain("return 'VM images'");
  });

  it('lets the service own quick-create session names and resources', () => {
    const quickCreateSources = [dashboard, appShell].join('\n');
    expect(quickCreateSources).not.toContain('generatedVmName');
    expect(quickCreateSources).not.toContain('ram_mb');
    expect(quickCreateSources).not.toContain('cpus:');
    expect(dashboard).toContain('vmStore.provision({ persistent: true })');
    expect(appShell).toContain('vmStore.openCreateModal()');
  });

  it('customizes a session from the service defaults', () => {
    expect(createDialog).toContain('const DEFAULT_RAM_MB = 12288');
    expect(createDialog).toContain('const DEFAULT_CPUS = 4');
    expect(createDialog).toContain('disabled={creating}');
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

  it('keeps active toolbar stats live and splits input, thinking, and output tokens', () => {
    expect(vmStore).not.toContain('api.getVmInfo(vm.id)');
    expect(vmStore).toContain('api.getVmStatsSummary(id)');
    expect(toolbar).toContain('vmStore.refreshActiveStats(id)');
    expect(toolbar).toContain('total_input_tokens');
    expect(toolbar).toContain('total_thinking_tokens');
    expect(toolbar).toContain('total_output_tokens');
    expect(toolbar).toContain('in /');
    expect(toolbar).toContain('think /');
    expect(toolbar).toContain('out');
    expect(toolbar).not.toContain('} tok</span>');
  });

  it('uses session wording in stats subtitles', () => {
    expect(stats).toContain('Session {vmId} ledger');
    expect(stats).not.toContain('Session {vmId} database');
    expect(stats).not.toContain('VM {vmId} session database');
  });
});
