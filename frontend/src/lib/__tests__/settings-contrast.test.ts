// WCAG AA contrast tests for settings UI colors.
// Verifies that warning text, badges, and status indicators
// meet the 4.5:1 minimum contrast ratio against their backgrounds.

import { describe, it, expect } from 'vitest';
import { contrastRatio } from '../terminal/themes.ts';

const MIN_CONTRAST = 4.5; // WCAG AA normal text

// Surface colors from global.css
const LIGHT_BG = '#ffffff';       // --background (card bg)
const LIGHT_BG_1 = '#f4f3f2';    // --background-1 (recessed, card headers)

const DARK_BG = '#282828';        // dark --background
const DARK_LAYER = '#3c3c3c';     // dark --layer (card bg)

// Warning colors (amber-700 light / amber-400 dark) -- used for lint warnings, "required" badges
const AMBER_700 = '#b45309';
const AMBER_400 = '#fbbf24';
const AMBER_100 = '#fef3c7';     // badge background

// Error text colors (red-700 light / red-400 dark) -- used for lint errors
const RED_700 = '#b91c1c';
const RED_300 = '#fca5a5';

// Status colors (green-700 light / green-400 dark) -- used for "Allowed" status
const GREEN_700 = '#15803d';
const GREEN_400 = '#4ade80';

// MCP badge colors (green-100/green-900 bg with green-700/green-400 text)
const GREEN_100 = '#dcfce7';

describe('settings warning text contrast', () => {
  describe('light mode', () => {
    it('amber-700 warning text on white card bg >= 4.5:1', () => {
      const ratio = contrastRatio(LIGHT_BG, AMBER_700);
      expect(ratio, `amber-700 on white = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    });

    it('amber-700 warning text on bg-background-1 >= 4.5:1', () => {
      const ratio = contrastRatio(LIGHT_BG_1, AMBER_700);
      expect(ratio, `amber-700 on bg-1 = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    });

    it('amber-700 on amber-100 badge bg >= 4.5:1', () => {
      const ratio = contrastRatio(AMBER_100, AMBER_700);
      expect(ratio, `amber-700 on amber-100 = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    });

    it('red-700 error text on white card bg >= 4.5:1', () => {
      const ratio = contrastRatio(LIGHT_BG, RED_700);
      expect(ratio, `red-700 on white = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    });

    it('green-700 status text on white card bg >= 4.5:1', () => {
      const ratio = contrastRatio(LIGHT_BG, GREEN_700);
      expect(ratio, `green-700 on white = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    });

    it('green-700 on green-100 badge bg >= 4.5:1', () => {
      const ratio = contrastRatio(GREEN_100, GREEN_700);
      expect(ratio, `green-700 on green-100 = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    });
  });

  describe('dark mode', () => {
    it('amber-400 warning text on dark bg >= 4.5:1', () => {
      const ratio = contrastRatio(DARK_BG, AMBER_400);
      expect(ratio, `amber-400 on dark bg = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    });

    it('amber-400 warning text on dark layer >= 4.5:1', () => {
      const ratio = contrastRatio(DARK_LAYER, AMBER_400);
      expect(ratio, `amber-400 on dark layer = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    });

    it('red-400 error text on dark bg >= 4.5:1', () => {
      const ratio = contrastRatio(DARK_BG, RED_300);
      expect(ratio, `red-400 on dark bg = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    });

    it('red-400 error text on dark layer >= 4.5:1', () => {
      const ratio = contrastRatio(DARK_LAYER, RED_300);
      expect(ratio, `red-400 on dark layer = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    });

    it('green-400 status text on dark bg >= 4.5:1', () => {
      const ratio = contrastRatio(DARK_BG, GREEN_400);
      expect(ratio, `green-400 on dark bg = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    });
  });
});
