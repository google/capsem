import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const source = readFileSync(
  new URL('../components/settings/PluginSection.svelte', import.meta.url),
  'utf8',
);
const settingsPage = readFileSync(
  new URL('../components/shell/SettingsPage.svelte', import.meta.url),
  'utf8',
);

describe('PluginSection route contract', () => {
  it('lives in Settings and loads service-wide plugins without a scope argument', () => {
    expect(settingsPage).toContain('<PluginSection />');
    expect(source).toContain('await listPlugins()');
    expect(source).toContain('await updatePlugin(plugin.id, { mode })');
    expect(source).toContain('<CredentialInventory />');
    expect(source).not.toContain('reloadCredentialBrokerStore');
    expect(source).not.toContain('$props');
    expect(source).not.toContain('scope');
  });

  it('renders plugin modes from the typed enum with recognizable icons', () => {
    expect(source).toContain('const MODE_META: Record<PluginMode');
    expect(source).toContain('allow:');
    expect(source).toContain('ask:');
    expect(source).toContain('block:');
    expect(source).toContain('rewrite:');
    expect(source).toContain('disable:');
    expect(source).toContain('<modeMeta.icon');
  });

  it('keeps disabled plugins visible but inactive instead of hiding their mode', () => {
    expect(source).toContain("plugin.config.mode === 'disable'");
    expect(source).toContain('bg-muted/20 opacity-70');
    expect(source).toContain("label: 'Disabled'");
  });

  it('renders other plugin-owned event families while credential details are read-only', () => {
    expect(source).toContain('plugin.capabilities.event_families');
    expect(source).toContain("{#if plugin.id === 'credential_broker'}");
    expect(source).not.toContain('CredentialInjectionForm');
    expect(source).not.toContain('ConnectionSection');
  });
});
