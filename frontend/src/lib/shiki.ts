// Shared Shiki highlighter singleton.
// Used by FileContent.svelte, FileEditorControl.svelte, StatsView.svelte.
//
// Uses `shiki/core` with fully on-demand language and theme loading.
// The startup bundle contains only the core runtime + the JS regex
// engine (~30 KB). Each grammar and theme lives in its own chunk,
// fetched the first time it's needed and retained for the session.
// Prefer the `highlightCode()` helper below -- it wraps ensure + render
// into one call so callers don't need to orchestrate the two promises.

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

// Every supported language maps to a `() => import(...)` so Vite emits
// each grammar as its own chunk. Adding a language is a one-liner here
// plus the detection map at the bottom of this file.
const LANG_LOADERS: Record<string, () => Promise<unknown>> = {
  rust:       () => import('@shikijs/langs/rust'),
  toml:       () => import('@shikijs/langs/toml'),
  markdown:   () => import('@shikijs/langs/markdown'),
  json:       () => import('@shikijs/langs/json'),
  typescript: () => import('@shikijs/langs/typescript'),
  javascript: () => import('@shikijs/langs/javascript'),
  python:     () => import('@shikijs/langs/python'),
  bash:       () => import('@shikijs/langs/bash'),
  yaml:       () => import('@shikijs/langs/yaml'),
  html:       () => import('@shikijs/langs/html'),
  css:        () => import('@shikijs/langs/css'),
  sql:        () => import('@shikijs/langs/sql'),
  go:         () => import('@shikijs/langs/go'),
  c:          () => import('@shikijs/langs/c'),
  cpp:        () => import('@shikijs/langs/cpp'),
  java:       () => import('@shikijs/langs/java'),
  xml:        () => import('@shikijs/langs/xml'),
  dockerfile: () => import('@shikijs/langs/dockerfile'),
  makefile:   () => import('@shikijs/langs/makefile'),
  ini:        () => import('@shikijs/langs/ini'),
  csv:        () => import('@shikijs/langs/csv'),
  svelte:     () => import('@shikijs/langs/svelte'),
  tsx:        () => import('@shikijs/langs/tsx'),
  jsx:        () => import('@shikijs/langs/jsx'),
  graphql:    () => import('@shikijs/langs/graphql'),
  ruby:       () => import('@shikijs/langs/ruby'),
  php:        () => import('@shikijs/langs/php'),
  swift:      () => import('@shikijs/langs/swift'),
  kotlin:     () => import('@shikijs/langs/kotlin'),
  lua:        () => import('@shikijs/langs/lua'),
  r:          () => import('@shikijs/langs/r'),
};

const THEME_LOADERS: Record<ShikiThemeId, () => Promise<unknown>> = {
  'github-dark-default':  () => import('@shikijs/themes/github-dark-default'),
  'github-light-default': () => import('@shikijs/themes/github-light-default'),
  'github-light':         () => import('@shikijs/themes/github-light'),
  'one-dark-pro':         () => import('@shikijs/themes/one-dark-pro'),
  'one-light':            () => import('@shikijs/themes/one-light'),
  'dracula':              () => import('@shikijs/themes/dracula'),
  'catppuccin-mocha':     () => import('@shikijs/themes/catppuccin-mocha'),
  'catppuccin-latte':     () => import('@shikijs/themes/catppuccin-latte'),
  'monokai':              () => import('@shikijs/themes/monokai'),
  'gruvbox-dark-medium':  () => import('@shikijs/themes/gruvbox-dark-medium'),
  'gruvbox-light-medium': () => import('@shikijs/themes/gruvbox-light-medium'),
  'solarized-dark':       () => import('@shikijs/themes/solarized-dark'),
  'solarized-light':      () => import('@shikijs/themes/solarized-light'),
  'nord':                 () => import('@shikijs/themes/nord'),
  'rose-pine':            () => import('@shikijs/themes/rose-pine'),
  'rose-pine-dawn':       () => import('@shikijs/themes/rose-pine-dawn'),
  'tokyo-night':          () => import('@shikijs/themes/tokyo-night'),
  'kanagawa-wave':        () => import('@shikijs/themes/kanagawa-wave'),
  'kanagawa-lotus':       () => import('@shikijs/themes/kanagawa-lotus'),
  'everforest-dark':      () => import('@shikijs/themes/everforest-dark'),
  'everforest-light':     () => import('@shikijs/themes/everforest-light'),
};

let instance: HighlighterCore | null = null;
let initPromise: Promise<HighlighterCore> | null = null;

/** Get (or lazily create) the shared Shiki highlighter. The returned
 *  highlighter has no languages and no themes registered -- callers
 *  should use `highlightCode()` or `ensureShikiLang` / `ensureShikiTheme`
 *  before calling `codeToHtml`. */
export async function getShikiHighlighter(): Promise<HighlighterCore> {
  if (instance) return instance;
  if (initPromise) return initPromise;
  initPromise = createHighlighterCore({
    themes: [],
    langs: [],
    engine: createJavaScriptRegexEngine(),
  }).then(h => {
    instance = h;
    return h;
  });
  return initPromise;
}

const pendingLangs = new Map<string, Promise<void>>();
const pendingThemes = new Map<string, Promise<void>>();

type GrammarModule = { default: unknown };

/** Ensure a Shiki grammar is loaded. No-op if already registered or if
 *  the lang isn't one we support (falls through to 'text' at render
 *  time). Concurrent calls share one network fetch. */
export async function ensureShikiLang(lang: string): Promise<void> {
  const loader = LANG_LOADERS[lang];
  if (!loader) return;
  const hl = await getShikiHighlighter();
  if (hl.getLoadedLanguages().includes(lang)) return;
  let p = pendingLangs.get(lang);
  if (!p) {
    p = loader().then(mod => hl.loadLanguage((mod as GrammarModule).default as never));
    pendingLangs.set(lang, p);
  }
  return p;
}

/** Ensure a Shiki theme is loaded. See `ensureShikiLang` for semantics. */
export async function ensureShikiTheme(theme: ShikiThemeId): Promise<void> {
  const loader = THEME_LOADERS[theme];
  if (!loader) return;
  const hl = await getShikiHighlighter();
  if (hl.getLoadedThemes().includes(theme)) return;
  let p = pendingThemes.get(theme);
  if (!p) {
    p = loader().then(mod => hl.loadTheme((mod as GrammarModule).default as never));
    pendingThemes.set(theme, p);
  }
  return p;
}

/** Highlight code to HTML. Loads the grammar and theme on first use and
 *  caches them for the session. Falls back to an HTML-escaped `<pre>`
 *  if the requested lang isn't supported (prevents a Shiki throw from
 *  breaking the containing view). */
export async function highlightCode(
  code: string,
  lang: string,
  theme: ShikiThemeId,
): Promise<string> {
  const hl = await getShikiHighlighter();
  const resolvedLang = LANG_LOADERS[lang] ? lang : 'text';
  await Promise.all([
    resolvedLang === 'text' ? Promise.resolve() : ensureShikiLang(resolvedLang),
    ensureShikiTheme(theme),
  ]);
  return hl.codeToHtml(code, { lang: resolvedLang, theme });
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
    sql: 'sql', go: 'go', c: 'c', h: 'c', cpp: 'cpp', hpp: 'cpp', cc: 'cpp', cxx: 'cpp',
    java: 'java', kt: 'kotlin', swift: 'swift', rb: 'ruby', php: 'php',
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
