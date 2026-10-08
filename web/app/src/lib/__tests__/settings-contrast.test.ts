// Guard the real semantic inputs. UI qualification measures numeric contrast
// from rendered light/dark colors rather than unrelated palette constants.
import { readFileSync } from 'node:fs';
import { expect, it } from 'vitest';

it.each(['McpSection', 'PluginSection'])('%s uses neutral-surface error text rather than button foreground', name => {
  const source = readFileSync(new URL(`../components/settings/${name}.svelte`, import.meta.url), 'utf8');
  expect(source).toContain('text-destructive-text');
  expect(source).not.toContain('text-destructive-foreground');
});

it('uses the owned error text token for settings load failures', () => {
  const source = readFileSync(new URL('../components/shell/SettingsPage.svelte', import.meta.url), 'utf8');
  expect(source).toContain('text-destructive-text');
  expect(source).not.toContain('text-destructive-foreground');
});
