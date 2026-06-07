// Global theme store. Three independent axes:
//   1. UI mode (light/dark) -- controls Preline/Tailwind shell
//   2. Preline theme (ocean, moon, ...) -- controls chrome/shell colors
//   3. Terminal theme family (dracula, nord, ...) -- controls xterm.js in all iframes
//
// All persist in localStorage (parent frame only -- iframes cannot access it).
// No $effect() here -- this is a module-level singleton, effects would be orphaned.
// localStorage sync happens in the setters.

import { FAMILY_NAMES, DEFAULT_FAMILY, resolveThemeKey } from '../terminal/themes';

const UI_MODE_KEY = 'capsem-ui-mode';
const TERMINAL_THEME_KEY = 'capsem-terminal-theme';
const PRELINE_THEME_KEY = 'capsem-preline-theme';
const FONT_SIZE_KEY = 'capsem-font-size';
const FONT_FAMILY_KEY = 'capsem-font-family';
const UI_FONT_SIZE_KEY = 'capsem-ui-font-size';

type UiMode = 'light' | 'dark';
type UiModePref = 'auto' | 'light' | 'dark';

function systemMode(): UiMode {
  if (typeof window !== 'undefined' && window.matchMedia?.('(prefers-color-scheme: light)').matches) {
    return 'light';
  }
  return 'dark';
}

function resolveMode(pref: UiModePref): UiMode {
  return pref === 'auto' ? systemMode() : pref;
}

// -- Preline themes (value = data-theme attribute, '' = default/no attribute) --

export const PRELINE_THEMES = [
  { value: '',                label: 'Default',    color: '#2563eb' },
  { value: 'theme-ocean',    label: 'Ocean',      color: '#0891b2' },
  { value: 'theme-moon',     label: 'Moon',       color: '#1f2937' },
  { value: 'theme-harvest',  label: 'Harvest',    color: '#b45309' },
  { value: 'theme-retro',    label: 'Retro',      color: '#ec4899' },
  { value: 'theme-autumn',   label: 'Autumn',     color: '#ca8a04' },
  { value: 'theme-bubblegum',label: 'Bubblegum',  color: '#db2777' },
  { value: 'theme-cashmere', label: 'Cashmere',   color: '#a855f7' },
  { value: 'theme-olive',    label: 'Olive',      color: '#4d7c0f' },
] as const;

const PRELINE_THEME_VALUES = PRELINE_THEMES.map(t => t.value) as readonly string[];

// -- localStorage load/save helpers --

function loadUiModePref(): UiModePref {
  try {
    const stored = localStorage.getItem(UI_MODE_KEY);
    if (stored === 'light' || stored === 'dark' || stored === 'auto') return stored;
  } catch {
    // localStorage unavailable (sandboxed iframe, SSR)
  }
  return 'auto';
}

function loadTerminalTheme(): string {
  try {
    const stored = localStorage.getItem(TERMINAL_THEME_KEY);
    if (stored && FAMILY_NAMES.includes(stored)) return stored;
  } catch {
    // localStorage unavailable
  }
  return DEFAULT_FAMILY;
}

function loadPrelineTheme(): string {
  try {
    const stored = localStorage.getItem(PRELINE_THEME_KEY);
    if (stored && PRELINE_THEME_VALUES.includes(stored)) return stored;
  } catch {
    // localStorage unavailable
  }
  return '';
}

function applyMode(effective: UiMode): void {
  if (typeof document !== 'undefined') {
    document.documentElement.classList.toggle('dark', effective === 'dark');
  }
}

function saveUiModePref(pref: UiModePref): void {
  try { localStorage.setItem(UI_MODE_KEY, pref); } catch { /* ignore */ }
  applyMode(resolveMode(pref));
}

function saveTerminalTheme(name: string): void {
  try { localStorage.setItem(TERMINAL_THEME_KEY, name); } catch { /* ignore */ }
}

function savePrelineTheme(theme: string): void {
  try { localStorage.setItem(PRELINE_THEME_KEY, theme); } catch { /* ignore */ }
  if (typeof document !== 'undefined') {
    if (theme) {
      document.documentElement.dataset.theme = theme;
    } else {
      delete document.documentElement.dataset.theme;
    }
  }
}

// -- Font settings --

export const FONT_SIZES = [10, 11, 12, 13, 14, 15, 16, 18, 20] as const;
export const DEFAULT_FONT_SIZE = 14;

// Bundled monospace fonts (loaded via @font-face in global.css, zero external deps).
export const FONT_FAMILIES = [
  { value: '"Google Sans Code", ui-monospace, monospace', label: 'Google Sans Code' },
  { value: '"JetBrains Mono", ui-monospace, monospace', label: 'JetBrains Mono' },
  { value: '"Fira Code", ui-monospace, monospace', label: 'Fira Code' },
  { value: '"Cascadia Code", ui-monospace, monospace', label: 'Cascadia Code' },
  { value: '"Inconsolata", ui-monospace, monospace', label: 'Inconsolata' },
  { value: '"Hack", ui-monospace, monospace', label: 'Hack' },
  { value: '"Space Mono", ui-monospace, monospace', label: 'Space Mono' },
  { value: '"Ubuntu Mono", ui-monospace, monospace', label: 'Ubuntu Mono' },
  { value: '"SF Mono", SFMono-Regular, ui-monospace, monospace', label: 'SF Mono' },
  { value: 'Menlo, ui-monospace, monospace', label: 'Menlo' },
  { value: 'Monaco, ui-monospace, monospace', label: 'Monaco' },
] as const;
export const DEFAULT_FONT_FAMILY = FONT_FAMILIES[0].value;

function loadFontSize(): number {
  try {
    const stored = localStorage.getItem(FONT_SIZE_KEY);
    if (stored) {
      const n = parseInt(stored, 10);
      if (FONT_SIZES.includes(n as any)) return n;
    }
  } catch { /* ignore */ }
  return DEFAULT_FONT_SIZE;
}

function loadFontFamily(): string {
  try {
    const stored = localStorage.getItem(FONT_FAMILY_KEY);
    if (stored && FONT_FAMILIES.some(f => f.value === stored)) return stored;
  } catch { /* ignore */ }
  return DEFAULT_FONT_FAMILY;
}

function saveFontSize(size: number): void {
  try { localStorage.setItem(FONT_SIZE_KEY, String(size)); } catch { /* ignore */ }
}

function saveFontFamily(family: string): void {
  try { localStorage.setItem(FONT_FAMILY_KEY, family); } catch { /* ignore */ }
}

// -- UI font size --

export const UI_FONT_SIZES = [12, 13, 14, 15, 16, 18] as const;
export const DEFAULT_UI_FONT_SIZE = 14;

function loadUiFontSize(): number {
  try {
    const stored = localStorage.getItem(UI_FONT_SIZE_KEY);
    if (stored) {
      const n = parseInt(stored, 10);
      if ((UI_FONT_SIZES as readonly number[]).includes(n)) return n;
    }
  } catch { /* ignore */ }
  return DEFAULT_UI_FONT_SIZE;
}

function saveUiFontSize(size: number): void {
  try { localStorage.setItem(UI_FONT_SIZE_KEY, String(size)); } catch { /* ignore */ }
  if (typeof document !== 'undefined') {
    document.documentElement.style.fontSize = size + 'px';
  }
}

// -- Store --

class ThemeStore {
  modePref = $state<UiModePref>(loadUiModePref());
  // Effective mode: tracks system preference when modePref is 'auto'
  #systemMode = $state<UiMode>(systemMode());
  terminalTheme = $state<string>(loadTerminalTheme());
  prelineTheme = $state<string>(loadPrelineTheme());
  fontSize = $state<number>(loadFontSize());
  fontFamily = $state<string>(loadFontFamily());
  uiFontSize = $state<number>(loadUiFontSize());

  constructor() {
    if (typeof document !== 'undefined') {
      // Apply initial state to DOM
      applyMode(this.mode);
      if (this.prelineTheme) {
        document.documentElement.dataset.theme = this.prelineTheme;
      }
      if (this.uiFontSize !== DEFAULT_UI_FONT_SIZE) {
        document.documentElement.style.fontSize = this.uiFontSize + 'px';
      }
    }
    // Listen for system color scheme changes (for auto mode)
    if (typeof window !== 'undefined') {
      window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', (e) => {
        this.#systemMode = e.matches ? 'dark' : 'light';
        if (this.modePref === 'auto') applyMode(this.#systemMode);
      });
    }
  }

  /** Effective mode (resolved from pref + system). */
  get mode(): UiMode {
    return this.modePref === 'auto' ? this.#systemMode : this.modePref;
  }

  /** Resolved terminal theme key (e.g. 'dracula' + dark -> 'dracula'). */
  get resolvedTerminalTheme(): string {
    return resolveThemeKey(this.terminalTheme, this.mode);
  }

  setMode(pref: UiModePref): void {
    this.modePref = pref;
    saveUiModePref(pref);
  }

  toggleMode(): void {
    // Cycle: auto -> light -> dark -> auto
    const next: Record<UiModePref, UiModePref> = { auto: 'light', light: 'dark', dark: 'auto' };
    this.setMode(next[this.modePref]);
  }

  setTerminalTheme(name: string): void {
    if (FAMILY_NAMES.includes(name)) {
      this.terminalTheme = name;
      saveTerminalTheme(name);
    }
  }

  setPrelineTheme(theme: string): void {
    if (theme === '' || PRELINE_THEME_VALUES.includes(theme)) {
      this.prelineTheme = theme;
      savePrelineTheme(theme);
    }
  }

  setFontSize(size: number): void {
    if ((FONT_SIZES as readonly number[]).includes(size)) {
      this.fontSize = size;
      saveFontSize(size);
    }
  }

  setFontFamily(family: string): void {
    if (FONT_FAMILIES.some(f => f.value === family)) {
      this.fontFamily = family;
      saveFontFamily(family);
    }
  }

  setUiFontSize(size: number): void {
    if ((UI_FONT_SIZES as readonly number[]).includes(size)) {
      this.uiFontSize = size;
      saveUiFontSize(size);
    }
  }
}

export const themeStore = new ThemeStore();
