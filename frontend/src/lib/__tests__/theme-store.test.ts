import { describe, it, expect, beforeEach, vi } from 'vitest';

// Mock browser globals before importing the store module.
// The store initializes on import (module-level singleton), so mocks must exist first.

const storage = new Map<string, string>();
const mockLocalStorage = {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => storage.set(key, value),
  removeItem: (key: string) => storage.delete(key),
  clear: () => storage.clear(),
  get length() { return storage.size; },
  key: (_i: number) => null as string | null,
};

const darkClasses = new Set<string>();
const mockDocumentElement = {
  classList: {
    toggle: (cls: string, force?: boolean) => {
      if (force) darkClasses.add(cls);
      else darkClasses.delete(cls);
    },
    contains: (cls: string) => darkClasses.has(cls),
  },
  dataset: {} as Record<string, string | undefined>,
  style: { fontSize: '' } as CSSStyleDeclaration,
};

let systemDarkMatches = true;
const changeListeners: Array<(e: { matches: boolean }) => void> = [];

vi.stubGlobal('localStorage', mockLocalStorage);
vi.stubGlobal('document', {
  documentElement: mockDocumentElement,
  fonts: { ready: Promise.resolve() },
});
vi.stubGlobal('window', {
  matchMedia: (query: string) => ({
    matches: query.includes('light') ? !systemDarkMatches : systemDarkMatches,
    addEventListener: (_event: string, cb: (e: { matches: boolean }) => void) => {
      changeListeners.push(cb);
    },
    removeEventListener: () => {},
  }),
});

// Now import -- the singleton initializes with our mocks
const { themeStore, PRELINE_THEMES, FONT_SIZES, FONT_FAMILIES, DEFAULT_FONT_SIZE, DEFAULT_FONT_FAMILY } =
  await import('../stores/theme.svelte.ts');

describe('themeStore', () => {
  beforeEach(() => {
    storage.clear();
    darkClasses.clear();
    delete mockDocumentElement.dataset.theme;
    mockDocumentElement.style.fontSize = '';
    systemDarkMatches = true;
    changeListeners.length = 0;

    // Reset store to defaults
    themeStore.setMode('auto');
    themeStore.setTerminalTheme('default');
    themeStore.setPrelineTheme('');
    themeStore.setFontSize(DEFAULT_FONT_SIZE);
    themeStore.setFontFamily(DEFAULT_FONT_FAMILY);
    themeStore.setUiFontSize(14);
  });

  // -- defaults --

  it('starts with auto mode', () => {
    expect(themeStore.modePref).toBe('auto');
  });

  it('resolves auto mode to system preference', () => {
    // systemDarkMatches = true, so mode should be dark
    expect(themeStore.mode).toBe('dark');
  });

  // -- setMode --

  it('setMode persists to localStorage', () => {
    themeStore.setMode('light');
    expect(storage.get('capsem-ui-mode')).toBe('light');
  });

  it('setMode light removes dark class', () => {
    darkClasses.add('dark');
    themeStore.setMode('light');
    expect(darkClasses.has('dark')).toBe(false);
  });

  it('setMode dark adds dark class', () => {
    themeStore.setMode('dark');
    expect(darkClasses.has('dark')).toBe(true);
  });

  // -- toggleMode --

  it('toggleMode cycles auto -> light -> dark -> auto', () => {
    expect(themeStore.modePref).toBe('auto');

    themeStore.toggleMode();
    expect(themeStore.modePref).toBe('light');

    themeStore.toggleMode();
    expect(themeStore.modePref).toBe('dark');

    themeStore.toggleMode();
    expect(themeStore.modePref).toBe('auto');
  });

  // -- setTerminalTheme --

  it('accepts valid terminal theme family', () => {
    themeStore.setTerminalTheme('dracula');
    expect(themeStore.terminalTheme).toBe('dracula');
    expect(storage.get('capsem-terminal-theme')).toBe('dracula');
  });

  it('ignores invalid terminal theme family', () => {
    themeStore.setTerminalTheme('default');
    themeStore.setTerminalTheme('nonexistent-theme');
    expect(themeStore.terminalTheme).toBe('default');
  });

  // -- setPrelineTheme --

  it('accepts valid Preline theme', () => {
    themeStore.setPrelineTheme('theme-ocean');
    expect(themeStore.prelineTheme).toBe('theme-ocean');
    expect(storage.get('capsem-preline-theme')).toBe('theme-ocean');
    expect(mockDocumentElement.dataset.theme).toBe('theme-ocean');
  });

  it('accepts empty string (default theme)', () => {
    themeStore.setPrelineTheme('theme-moon');
    themeStore.setPrelineTheme('');
    expect(themeStore.prelineTheme).toBe('');
  });

  it('ignores invalid Preline theme', () => {
    themeStore.setPrelineTheme('');
    themeStore.setPrelineTheme('theme-fantasy');
    expect(themeStore.prelineTheme).toBe('');
  });

  // -- setFontSize --

  it('accepts valid font size', () => {
    themeStore.setFontSize(16);
    expect(themeStore.fontSize).toBe(16);
    expect(storage.get('capsem-font-size')).toBe('16');
  });

  it('ignores invalid font size', () => {
    themeStore.setFontSize(14);
    themeStore.setFontSize(99);
    expect(themeStore.fontSize).toBe(14);
  });

  // -- setFontFamily --

  it('accepts valid font family', () => {
    const jetbrains = FONT_FAMILIES[1].value;
    themeStore.setFontFamily(jetbrains);
    expect(themeStore.fontFamily).toBe(jetbrains);
    expect(storage.get('capsem-font-family')).toBe(jetbrains);
  });

  it('ignores invalid font family', () => {
    themeStore.setFontFamily(DEFAULT_FONT_FAMILY);
    themeStore.setFontFamily('Comic Sans MS');
    expect(themeStore.fontFamily).toBe(DEFAULT_FONT_FAMILY);
  });

  // -- setUiFontSize --

  it('accepts valid UI font size', () => {
    themeStore.setUiFontSize(16);
    expect(themeStore.uiFontSize).toBe(16);
    expect(storage.get('capsem-ui-font-size')).toBe('16');
    expect(mockDocumentElement.style.fontSize).toBe('16px');
  });

  it('ignores invalid UI font size', () => {
    themeStore.setUiFontSize(14);
    themeStore.setUiFontSize(99);
    expect(themeStore.uiFontSize).toBe(14);
  });

  // -- exports --

  it('exports PRELINE_THEMES with 9 entries', () => {
    expect(PRELINE_THEMES).toHaveLength(9);
  });

  it('exports FONT_SIZES array', () => {
    expect(FONT_SIZES.length).toBeGreaterThan(0);
    expect(FONT_SIZES).toContain(14);
  });

  it('exports FONT_FAMILIES array', () => {
    expect(FONT_FAMILIES.length).toBeGreaterThan(0);
  });
});
