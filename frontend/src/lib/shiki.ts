// Shared Shiki highlighter singleton.
// Used by FileContent.svelte and FileEditorControl.svelte.
//
// Uses `shiki/core` with explicit language/theme imports so the bundler
// only includes the grammars we actually ship. Importing from `'shiki'`
// would pull the full bundled-languages registry (235 langs) -- Vite then
// code-splits every one, producing >500 KB chunks for languages we never
// use (emacs-lisp, wolfram, wasm, ...).

import { createHighlighterCore, type HighlighterCore } from 'shiki/core';
import { createJavaScriptRegexEngine } from 'shiki/engine/javascript';

export type ShikiThemeId =
  | 'github-dark-default' | 'github-light-default' | 'github-light'
  | 'one-dark-pro' | 'one-light'
  | 'dracula'
  | 'catppuccin-mocha' | 'catppuccin-latte'
  | 'monokai'
  | 'gruvbox-dark-medium' | 'gruvbox-light-medium'
  | 'solarized-dark' | 'solarized-light'
  | 'nord'
  | 'rose-pine' | 'rose-pine-dawn'
  | 'tokyo-night'
  | 'kanagawa-wave' | 'kanagawa-lotus'
  | 'everforest-dark' | 'everforest-light';

export type ShikiHighlighter = HighlighterCore;

// Map terminal theme families to Shiki theme IDs
export const SHIKI_THEMES: Record<string, { dark: ShikiThemeId; light: ShikiThemeId }> = {
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

let instance: HighlighterCore | null = null;
let initPromise: Promise<HighlighterCore> | null = null;

/** Get (or lazily create) the shared Shiki highlighter. */
export async function getShikiHighlighter(): Promise<HighlighterCore> {
  if (instance) return instance;
  if (initPromise) return initPromise;
  initPromise = createHighlighterCore({
    themes: [
      import('@shikijs/themes/github-dark-default'),
      import('@shikijs/themes/github-light-default'),
      import('@shikijs/themes/github-light'),
      import('@shikijs/themes/one-dark-pro'),
      import('@shikijs/themes/one-light'),
      import('@shikijs/themes/dracula'),
      import('@shikijs/themes/catppuccin-mocha'),
      import('@shikijs/themes/catppuccin-latte'),
      import('@shikijs/themes/monokai'),
      import('@shikijs/themes/gruvbox-dark-medium'),
      import('@shikijs/themes/gruvbox-light-medium'),
      import('@shikijs/themes/solarized-dark'),
      import('@shikijs/themes/solarized-light'),
      import('@shikijs/themes/nord'),
      import('@shikijs/themes/rose-pine'),
      import('@shikijs/themes/rose-pine-dawn'),
      import('@shikijs/themes/tokyo-night'),
      import('@shikijs/themes/kanagawa-wave'),
      import('@shikijs/themes/kanagawa-lotus'),
      import('@shikijs/themes/everforest-dark'),
      import('@shikijs/themes/everforest-light'),
    ],
    langs: [
      import('@shikijs/langs/rust'),
      import('@shikijs/langs/toml'),
      import('@shikijs/langs/markdown'),
      import('@shikijs/langs/json'),
      import('@shikijs/langs/typescript'),
      import('@shikijs/langs/javascript'),
      import('@shikijs/langs/python'),
      import('@shikijs/langs/bash'),
      import('@shikijs/langs/yaml'),
      import('@shikijs/langs/html'),
      import('@shikijs/langs/css'),
      import('@shikijs/langs/sql'),
      import('@shikijs/langs/go'),
      import('@shikijs/langs/c'),
      // cpp grammar is 419 KB of JSON that bundles to 620 KB; every Shiki
      // host grammar (ruby, php, ...) re-imports it for inline-assembly
      // heredocs, blowing past Vite's 500 KB chunk budget. .cpp/.hpp fall
      // back to `c` grammar, which covers keywords/strings/comments and
      // loses templates/namespaces -- acceptable for a sandbox file viewer.
      import('@shikijs/langs/java'),
      import('@shikijs/langs/xml'),
      import('@shikijs/langs/dockerfile'),
      import('@shikijs/langs/makefile'),
      import('@shikijs/langs/ini'),
      import('@shikijs/langs/csv'),
      import('@shikijs/langs/svelte'),
      import('@shikijs/langs/tsx'),
      import('@shikijs/langs/jsx'),
      import('@shikijs/langs/graphql'),
      // ruby embeds 12 sub-grammars (html, cpp, graphql, sql, haml, ...)
      // which bundles to ~676 KB. Plain text is acceptable for .rb files
      // in a sandbox file viewer.
      import('@shikijs/langs/php'),
      import('@shikijs/langs/swift'),
      import('@shikijs/langs/kotlin'),
      import('@shikijs/langs/lua'),
      import('@shikijs/langs/r'),
    ],
    engine: createJavaScriptRegexEngine(),
  }).then(h => {
    instance = h;
    return h;
  });
  return initPromise;
}

/** Resolve the Shiki theme ID for the current terminal theme + mode. */
export function resolveShikiTheme(terminalTheme: string, mode: 'light' | 'dark'): ShikiThemeId {
  const entry = SHIKI_THEMES[terminalTheme] ?? SHIKI_THEMES['default'];
  return mode === 'dark' ? entry.dark : entry.light;
}

/** Detect language from file extension, filetype hint, or content sniffing. */
export function detectShikiLang(filetypeOrPath: string, content?: string): string {
  const ext = filetypeOrPath.includes('.') ? filetypeOrPath.split('.').pop()?.toLowerCase() ?? '' : filetypeOrPath.toLowerCase();
  const map: Record<string, string> = {
    rs: 'rust', toml: 'toml', md: 'markdown', json: 'json', jsonc: 'json',
    ts: 'typescript', tsx: 'tsx', js: 'javascript', jsx: 'jsx',
    py: 'python', sh: 'bash', bash: 'bash', zsh: 'bash',
    yaml: 'yaml', yml: 'yaml', xml: 'xml', svg: 'xml',
    html: 'html', htm: 'html', css: 'css', scss: 'css',
    sql: 'sql', go: 'go', c: 'c', h: 'c', cpp: 'c', hpp: 'c', cc: 'c', cxx: 'c',
    java: 'java', kt: 'kotlin', swift: 'swift', php: 'php',
    lua: 'lua', r: 'r', R: 'r', csv: 'csv',
    dockerfile: 'dockerfile', makefile: 'makefile',
    ini: 'ini', cfg: 'ini', env: 'ini',
    graphql: 'graphql', gql: 'graphql',
    svelte: 'svelte',
    conf: 'bash',
    // Magika labels (no dot, passed as-is)
    rust: 'rust', python: 'python', javascript: 'javascript', typescript: 'typescript',
    markdown: 'markdown',
  };
  const result = map[ext];
  if (result) return result;

  // Content sniffing fallback
  if (content) {
    const trimmed = content.trimStart();
    if (trimmed.startsWith('{') || trimmed.startsWith('[')) return 'json';
    if (trimmed.startsWith('<?xml') || trimmed.startsWith('<!DOCTYPE')) return 'xml';
    if (trimmed.startsWith('<')) return 'html';
    if (trimmed.startsWith('#!')) return 'bash';
  }

  return 'text';
}
