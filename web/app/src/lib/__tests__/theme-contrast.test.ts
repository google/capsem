import { describe, it, expect } from 'vitest';
import {
  parseHex,
  relativeLuminance,
  contrastRatio,
  TERMINAL_THEMES,
  THEME_NAMES,
} from '../terminal/themes.ts';

// -- Unit tests for the math --

describe('parseHex', () => {
  it('parses 6-digit hex', () => {
    expect(parseHex('#ff8000')).toEqual([255, 128, 0]);
  });

  it('parses 3-digit hex', () => {
    expect(parseHex('#f80')).toEqual([255, 136, 0]);
  });

  it('handles no hash', () => {
    expect(parseHex('000000')).toEqual([0, 0, 0]);
  });
});

describe('relativeLuminance', () => {
  it('black is 0', () => {
    expect(relativeLuminance('#000000')).toBeCloseTo(0, 4);
  });

  it('white is 1', () => {
    expect(relativeLuminance('#ffffff')).toBeCloseTo(1, 4);
  });

  it('mid-gray is ~0.2', () => {
    // sRGB #808080 -> linearized ~0.216
    expect(relativeLuminance('#808080')).toBeCloseTo(0.2159, 3);
  });
});

describe('contrastRatio', () => {
  it('black on white is 21:1', () => {
    expect(contrastRatio('#000000', '#ffffff')).toBeCloseTo(21, 0);
  });

  it('same color is 1:1', () => {
    expect(contrastRatio('#123456', '#123456')).toBeCloseTo(1, 2);
  });

  it('is commutative', () => {
    const r1 = contrastRatio('#ff0000', '#00ff00');
    const r2 = contrastRatio('#00ff00', '#ff0000');
    expect(r1).toBeCloseTo(r2, 6);
  });
});

// -- Contrast validation for every terminal theme --
// WCAG AA for normal text requires >= 4.5:1.
// Terminals use monospace at >= 14px so we use the large-text threshold of 3:1
// as the floor, but flag anything below 4.5:1 as a warning.

const MIN_CONTRAST = 4.5; // WCAG AA normal text

describe('terminal theme contrast', () => {
  for (const name of THEME_NAMES) {
    it(`${name}: foreground/background contrast >= ${MIN_CONTRAST}:1`, () => {
      const theme = TERMINAL_THEMES[name];
      const bg = theme.background!;
      const fg = theme.foreground!;
      const ratio = contrastRatio(bg, fg);
      expect(ratio, `${name} fg ${fg} on bg ${bg} = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    });

    it(`${name}: ANSI colors visible on background`, () => {
      const theme = TERMINAL_THEMES[name];
      const bg = theme.background!;
      const ansiColors = [
        { name: 'red', hex: theme.red },
        { name: 'green', hex: theme.green },
        { name: 'yellow', hex: theme.yellow },
        { name: 'blue', hex: theme.blue },
        { name: 'magenta', hex: theme.magenta },
        { name: 'cyan', hex: theme.cyan },
      ];
      for (const c of ansiColors) {
        if (!c.hex) continue;
        const ratio = contrastRatio(bg, c.hex);
        expect(ratio, `${name} ${c.name} ${c.hex} on bg ${bg} = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(MIN_CONTRAST);
      }
    });
  }
});
