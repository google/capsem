// Shared Shiki highlighter singleton.
// Used by FileContent.svelte and FileEditorControl.svelte.
// Avoids loading themes/languages twice.

import { createHighlighter, type Highlighter, type BundledTheme } from 'shiki';

// Map terminal theme families to Shiki theme IDs
export const SHIKI_THEMES: Record<string, { dark: BundledTheme; light: BundledTheme }> = {
  'default':     { dark: 'github-dark-default',  light: 'github-light-default' },
  'one':         { dark: 'one-dark-pro',          light: 'one-light' },
  'dracula':     { dark: 'dracula',               light: 'github-light' },
  'catppuccin':  { dark: 'catppuccin-mocha',      light: 'catppuccin-latte' },
  'monokai':     { dark: 'monokai',               light: 'github-light' },
  'gruvbox':     { dark: 'gruvbox-dark-medium',   light: 'gruvbox-light-medium' },
  'solarized':   { dark: 'solarized-dark',        light: 'solarized-light' },
  'nord':        { dark: 'nord',                  light: 'github-light' },
  'rose-pine':   { dark: 'rose-pine',             light: 'rose-pine-dawn' },
  'tokyo-night': { dark: 'tokyo-night',           light: 'github-light' },
  'kanagawa':    { dark: 'kanagawa-wave',         light: 'kanagawa-lotus' },
  'everforest':  { dark: 'everforest-dark',       light: 'everforest-light' },
};

const ALL_SHIKI_THEME_IDS = [...new Set(
  Object.values(SHIKI_THEMES).flatMap(t => [t.dark, t.light])
)];

const LANGS = ['rust', 'toml', 'markdown', 'json', 'typescript', 'javascript', 'python', 'bash', 'yaml'] as const;

let instance: Highlighter | null = null;
let initPromise: Promise<Highlighter> | null = null;

/** Get (or lazily create) the shared Shiki highlighter. */
export async function getShikiHighlighter(): Promise<Highlighter> {
  if (instance) return instance;
  if (initPromise) return initPromise;
  initPromise = createHighlighter({
    themes: ALL_SHIKI_THEME_IDS,
    langs: [...LANGS],
  }).then(h => {
    instance = h;
    return h;
  });
  return initPromise;
}

/** Resolve the Shiki theme ID for the current terminal theme + mode. */
export function resolveShikiTheme(terminalTheme: string, mode: 'light' | 'dark'): BundledTheme {
  const entry = SHIKI_THEMES[terminalTheme] ?? SHIKI_THEMES['default'];
  return mode === 'dark' ? entry.dark : entry.light;
}

/** Detect language from file extension or filetype hint. */
export function detectShikiLang(filetypeOrPath: string): string {
  const ext = filetypeOrPath.includes('.') ? filetypeOrPath.split('.').pop()?.toLowerCase() ?? '' : filetypeOrPath;
  const map: Record<string, string> = {
    rs: 'rust', toml: 'toml', md: 'markdown', json: 'json',
    ts: 'typescript', js: 'javascript', py: 'python',
    sh: 'bash', bash: 'bash', yaml: 'yaml', yml: 'yaml',
    conf: 'bash', // close enough for config files
  };
  return map[ext] ?? 'text';
}
