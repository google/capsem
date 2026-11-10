import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { FONT_FAMILIES } from '../stores/theme.svelte';
import { loadTerminalFont } from '../terminal/fonts';

function source(relativePath: string): string {
  return readFileSync(new URL(`../../${relativePath}`, import.meta.url), 'utf8');
}

// Fonts the host is expected to provide; everything else in the picker is bundled.
const SYSTEM_FONTS = new Set(['SF Mono', 'Menlo', 'Monaco']);

describe('terminal fonts', () => {
  it('declares every bundled terminal font in the shared font sheet', () => {
    const fonts = source('styles/fonts.css');
    const bundled = FONT_FAMILIES.map((font) => font.label).filter((label) => !SYSTEM_FONTS.has(label));
    expect(bundled.length).toBeGreaterThan(0);
    for (const label of bundled) {
      expect(fonts, label).toContain(`font-family: "${label}";`);
    }
  });

  it('loads the font sheet in the terminal iframe, which has no global.css', () => {
    // The terminal is its own document. Without the @font-face rules there,
    // a bundled font name resolves to nothing and the terminal silently
    // falls back to whatever the host calls ui-monospace.
    expect(source('pages/vm/terminal.astro')).toContain('import "../../styles/fonts.css";');
    expect(source('styles/global.css')).toContain('@import "./fonts.css";');
    expect(source('styles/global.css')).not.toContain('@font-face');
  });

  it('asks the browser for the chosen font before the terminal measures it', async () => {
    const load = vi.fn().mockResolvedValue([]);
    await loadTerminalFont({ load }, '"Google Sans Code", ui-monospace, monospace', 14);
    expect(load).toHaveBeenCalledWith('14px "Google Sans Code", ui-monospace, monospace');
  });

  it('still opens the terminal when a font cannot be loaded', async () => {
    const load = vi.fn().mockRejectedValue(new Error('network'));
    await expect(loadTerminalFont({ load }, 'Menlo, ui-monospace, monospace', 14)).resolves.toBeUndefined();
  });
});
