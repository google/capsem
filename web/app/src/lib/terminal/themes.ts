// Terminal color themes. Each maps to an xterm.js ITheme.
// Colors sourced from iTerm2-Color-Schemes (canonical palettes).
// Users pick a *family* (e.g. "dracula") in Settings; the resolved
// dark/light variant depends on UI mode.

import type { ITheme } from '@xterm/xterm';

// -- Theme families: each has a dark and light variant --
// `colors` = 5 representative hex colors for the swatch preview.

export interface ThemeFamily {
  name: string;
  label: string;
  dark: string;
  light: string;
  colors: [string, string, string, string, string];
}

export const THEME_FAMILIES: ThemeFamily[] = [
  { name: 'default',     label: 'Default',      dark: 'default-dark',        light: 'default-light',       colors: ['#0d1117', '#58a6ff', '#3fb950', '#d29922', '#bc8cff'] },
  { name: 'one',         label: 'One',           dark: 'atom-one-dark',       light: 'atom-one-light',      colors: ['#21252b', '#61afef', '#98c379', '#e5c07b', '#c678dd'] },
  { name: 'dracula',     label: 'Dracula',       dark: 'dracula',             light: 'dracula-light',       colors: ['#282a36', '#bd93f9', '#50fa7b', '#ff79c6', '#8be9fd'] },
  { name: 'catppuccin',  label: 'Catppuccin',    dark: 'catppuccin-mocha',    light: 'catppuccin-latte',    colors: ['#1e1e2e', '#89b4fa', '#a6e3a1', '#f38ba8', '#f5c2e7'] },
  { name: 'monokai',     label: 'Monokai',       dark: 'monokai',             light: 'monokai-light',       colors: ['#2d2a2e', '#ff6188', '#a9dc76', '#ffd866', '#ab9df2'] },
  { name: 'gruvbox',     label: 'Gruvbox',       dark: 'gruvbox-dark',        light: 'gruvbox-light',       colors: ['#282828', '#fb4934', '#b8bb26', '#fabd2f', '#83a598'] },
  { name: 'solarized',   label: 'Solarized',     dark: 'solarized-dark',      light: 'solarized-light',     colors: ['#002b36', '#3995d6', '#859900', '#b58900', '#de66a0'] },
  { name: 'nord',        label: 'Nord',           dark: 'nord',                light: 'nord-light',          colors: ['#2e3440', '#54697e', '#a3be8c', '#ebcb8b', '#796074'] },
  { name: 'rose-pine',   label: 'Rose Pine',     dark: 'rose-pine',           light: 'rose-pine-dawn',      colors: ['#191724', '#eb6f92', '#31748f', '#f6c177', '#c4a7e7'] },
  { name: 'tokyo-night', label: 'Tokyo Night',   dark: 'tokyo-night',         light: 'tokyo-night-light',   colors: ['#1a1b26', '#7aa2f7', '#9ece6a', '#e0af68', '#bb9af7'] },
  { name: 'kanagawa',    label: 'Kanagawa',      dark: 'kanagawa-wave',       light: 'kanagawa-lotus',      colors: ['#1f1f28', '#7e9cd8', '#76946a', '#c0a36e', '#957fb8'] },
  { name: 'everforest',  label: 'Everforest',    dark: 'everforest-dark',     light: 'everforest-light',    colors: ['#1e2326', '#4c6f6b', '#a7c080', '#dbbc7f', '#835e70'] },
];

export const FAMILY_NAMES = THEME_FAMILIES.map(f => f.name);

/** Resolve a family name + UI mode to a concrete theme key. */
export function resolveThemeKey(family: string, mode: 'light' | 'dark'): string {
  const f = THEME_FAMILIES.find(t => t.name === family);
  if (!f) return mode === 'light' ? 'default-light' : 'default-dark';
  return mode === 'light' ? f.light : f.dark;
}

// -- Individual themes --
// Source: tmp/iTerm2-Color-Schemes-master/xrdb/<Name>.xrdb
// Mapping: Ansi_0=black .. Ansi_15=brightWhite, Background/Foreground/Cursor/Selection.

export const TERMINAL_THEMES: Record<string, ITheme> = {

  // ---- Default (GitHub Dark Default / GitHub Light Default) ----

  'default-dark': {
    background: '#0d1117',
    foreground: '#e6edf3',
    cursor: '#2f81f7',
    selectionBackground: '#3b507080',
    black: '#484f58',
    red: '#ff7b72',
    green: '#3fb950',
    yellow: '#d29922',
    blue: '#58a6ff',
    magenta: '#bc8cff',
    cyan: '#39c5cf',
    white: '#b1bac4',
    brightBlack: '#6e7681',
    brightRed: '#ffa198',
    brightGreen: '#56d364',
    brightYellow: '#e3b341',
    brightBlue: '#79c0ff',
    brightMagenta: '#d2a8ff',
    brightCyan: '#56d4dd',
    brightWhite: '#ffffff',
  },

  'default-light': {
    background: '#ffffff',
    foreground: '#1f2328',
    cursor: '#0969da',
    selectionBackground: '#0969da30',
    black: '#24292f',
    red: '#cf222e',
    green: '#116329',
    yellow: '#4d2d00',
    blue: '#0969da',
    magenta: '#8250df',
    cyan: '#1b7c83',
    white: '#6e7781',
    brightBlack: '#57606a',
    brightRed: '#a40e26',
    brightGreen: '#1a7f37',
    brightYellow: '#633c01',
    brightBlue: '#218bff',
    brightMagenta: '#a475f9',
    brightCyan: '#3192aa',
    brightWhite: '#8c959f',
  },

  // ---- One (Atom One Dark / Atom One Light) ----

  'atom-one-dark': {
    background: '#21252b',
    foreground: '#abb2bf',
    cursor: '#abb2bf',
    selectionBackground: '#323844',
    black: '#21252b',
    red: '#e06c75',
    green: '#98c379',
    yellow: '#e5c07b',
    blue: '#61afef',
    magenta: '#c678dd',
    cyan: '#56b6c2',
    white: '#abb2bf',
    brightBlack: '#767676',
    brightRed: '#e06c75',
    brightGreen: '#98c379',
    brightYellow: '#e5c07b',
    brightBlue: '#61afef',
    brightMagenta: '#c678dd',
    brightCyan: '#56b6c2',
    brightWhite: '#abb2bf',
  },

  'atom-one-light': {
    background: '#f9f9f9',
    foreground: '#2a2c33',
    cursor: '#bbbbbb',
    selectionBackground: '#ededed',
    black: '#000000',
    red: '#d13a32',
    green: '#368032',
    yellow: '#806f4b',
    blue: '#2f5af3',
    magenta: '#950095',
    cyan: '#368032',
    white: '#bbbbbb',
    brightBlack: '#000000',
    brightRed: '#de3e35',
    brightGreen: '#3f953a',
    brightYellow: '#d2b67c',
    brightBlue: '#2f5af3',
    brightMagenta: '#a00095',
    brightCyan: '#3f953a',
    brightWhite: '#ffffff',
  },

  // ---- Dracula (Dracula.xrdb / crafted light) ----

  dracula: {
    background: '#282a36',
    foreground: '#f8f8f2',
    cursor: '#f8f8f2',
    selectionBackground: '#44475a',
    black: '#21222c',
    red: '#ff5555',
    green: '#50fa7b',
    yellow: '#f1fa8c',
    blue: '#bd93f9',
    magenta: '#ff79c6',
    cyan: '#8be9fd',
    white: '#f8f8f2',
    brightBlack: '#6272a4',
    brightRed: '#ff6e6e',
    brightGreen: '#69ff94',
    brightYellow: '#ffffa5',
    brightBlue: '#d6acff',
    brightMagenta: '#ff92df',
    brightCyan: '#a4ffff',
    brightWhite: '#ffffff',
  },

  'dracula-light': {
    background: '#f8f8f2',
    foreground: '#282a36',
    cursor: '#6272a4',
    selectionBackground: '#ccc5f0',
    black: '#282a36',
    red: '#d03245',
    green: '#1c823a',
    yellow: '#916a00',
    blue: '#7c5cc4',
    magenta: '#c23698',
    cyan: '#137a8a',
    white: '#6272a4',
    brightBlack: '#44475a',
    brightRed: '#e64747',
    brightGreen: '#2ea34d',
    brightYellow: '#b8860b',
    brightBlue: '#9b6ddf',
    brightMagenta: '#e05594',
    brightCyan: '#2a96b0',
    brightWhite: '#282a36',
  },

  // ---- Catppuccin (Mocha / Latte) ----

  'catppuccin-mocha': {
    background: '#1e1e2e',
    foreground: '#cdd6f4',
    cursor: '#f5e0dc',
    selectionBackground: '#585b70',
    black: '#45475a',
    red: '#f38ba8',
    green: '#a6e3a1',
    yellow: '#f9e2af',
    blue: '#89b4fa',
    magenta: '#f5c2e7',
    cyan: '#94e2d5',
    white: '#a6adc8',
    brightBlack: '#585b70',
    brightRed: '#f37799',
    brightGreen: '#89d88b',
    brightYellow: '#ebd391',
    brightBlue: '#74a8fc',
    brightMagenta: '#f2aede',
    brightCyan: '#6bd7ca',
    brightWhite: '#bac2de',
  },

  'catppuccin-latte': {
    background: '#eff1f5',
    foreground: '#4c4f69',
    cursor: '#dc8a78',
    selectionBackground: '#acb0be',
    black: '#5c5f77',
    red: '#d20f39',
    green: '#317b21',
    yellow: '#966014',
    blue: '#1d63ee',
    magenta: '#9d4f89',
    cyan: '#13767c',
    white: '#acb0be',
    brightBlack: '#6c6f85',
    brightRed: '#de293e',
    brightGreen: '#49af3d',
    brightYellow: '#eea02d',
    brightBlue: '#456eff',
    brightMagenta: '#fe85d8',
    brightCyan: '#2d9fa8',
    brightWhite: '#bcc0cc',
  },

  // ---- Monokai (Monokai Pro / Monokai Pro Light) ----

  monokai: {
    background: '#2d2a2e',
    foreground: '#fcfcfa',
    cursor: '#c1c0c0',
    selectionBackground: '#5b595c',
    black: '#2d2a2e',
    red: '#ff6188',
    green: '#a9dc76',
    yellow: '#ffd866',
    blue: '#fc9867',
    magenta: '#ab9df2',
    cyan: '#78dce8',
    white: '#fcfcfa',
    brightBlack: '#727072',
    brightRed: '#ff6188',
    brightGreen: '#a9dc76',
    brightYellow: '#ffd866',
    brightBlue: '#fc9867',
    brightMagenta: '#ab9df2',
    brightCyan: '#78dce8',
    brightWhite: '#fcfcfa',
  },

  'monokai-light': {
    background: '#faf4f2',
    foreground: '#29242a',
    cursor: '#706b6e',
    selectionBackground: '#bfb9ba',
    black: '#faf4f2',
    red: '#c13d64',
    green: '#1f7f55',
    yellow: '#a06008',
    blue: '#b64e28',
    magenta: '#7058be',
    cyan: '#187890',
    white: '#29242a',
    brightBlack: '#a59fa0',
    brightRed: '#e14775',
    brightGreen: '#269d69',
    brightYellow: '#cc7a0a',
    brightBlue: '#e16032',
    brightMagenta: '#7058be',
    brightCyan: '#1c8ca8',
    brightWhite: '#29242a',
  },

  // ---- Gruvbox (Dark / Light) ----

  'gruvbox-dark': {
    background: '#282828',
    foreground: '#ebdbb2',
    cursor: '#ebdbb2',
    selectionBackground: '#665c54',
    black: '#282828',
    red: '#de706c',
    green: '#98971a',
    yellow: '#d79921',
    blue: '#64999c',
    magenta: '#be7c9a',
    cyan: '#7c987c',
    white: '#a89984',
    brightBlack: '#928374',
    brightRed: '#fb4934',
    brightGreen: '#b8bb26',
    brightYellow: '#fabd2f',
    brightBlue: '#83a598',
    brightMagenta: '#d3869b',
    brightCyan: '#8ec07c',
    brightWhite: '#ebdbb2',
  },

  'gruvbox-light': {
    background: '#fbf1c7',
    foreground: '#3c3836',
    cursor: '#3c3836',
    selectionBackground: '#d5c4a1',
    black: '#fbf1c7',
    red: '#cc241d',
    green: '#6f6e13',
    yellow: '#8c6416',
    blue: '#3d7678',
    magenta: '#9d5777',
    cyan: '#4d744e',
    white: '#7c6f64',
    brightBlack: '#928374',
    brightRed: '#9d0006',
    brightGreen: '#79740e',
    brightYellow: '#b57614',
    brightBlue: '#076678',
    brightMagenta: '#8f3f71',
    brightCyan: '#427b58',
    brightWhite: '#3c3836',
  },

  // ---- Solarized (iTerm2 Solarized Dark / Light) ----

  'solarized-dark': {
    background: '#002b36',
    foreground: '#839496',
    cursor: '#839496',
    selectionBackground: '#073642',
    black: '#073642',
    red: '#e56866',
    green: '#859900',
    yellow: '#b58900',
    blue: '#5595c2',
    magenta: '#d6699d',
    cyan: '#559a95',
    white: '#eee8d5',
    brightBlack: '#335e69',
    brightRed: '#cb4b16',
    brightGreen: '#586e75',
    brightYellow: '#657b83',
    brightBlue: '#839496',
    brightMagenta: '#6c71c4',
    brightCyan: '#93a1a1',
    brightWhite: '#fdf6e3',
  },

  'solarized-light': {
    background: '#fdf6e3',
    foreground: '#5f747b',
    cursor: '#5f747b',
    selectionBackground: '#eee8d5',
    black: '#073642',
    red: '#cf2f2c',
    green: '#667500',
    yellow: '#8e6b00',
    blue: '#2074af',
    magenta: '#c7337a',
    cyan: '#207a74',
    white: '#bbb5a2',
    brightBlack: '#002b36',
    brightRed: '#cb4b16',
    brightGreen: '#586e75',
    brightYellow: '#657b83',
    brightBlue: '#839496',
    brightMagenta: '#6c71c4',
    brightCyan: '#93a1a1',
    brightWhite: '#fdf6e3',
  },

  // ---- Nord (Nord / Nord Light) ----

  nord: {
    background: '#2e3440',
    foreground: '#d8dee9',
    cursor: '#eceff4',
    selectionBackground: '#434c5e',
    black: '#3b4252',
    red: '#d08a91',
    green: '#a3be8c',
    yellow: '#ebcb8b',
    blue: '#81a1c1',
    magenta: '#b691af',
    cyan: '#88c0d0',
    white: '#e5e9f0',
    brightBlack: '#596377',
    brightRed: '#bf616a',
    brightGreen: '#a3be8c',
    brightYellow: '#ebcb8b',
    brightBlue: '#81a1c1',
    brightMagenta: '#b48ead',
    brightCyan: '#8fbcbb',
    brightWhite: '#eceff4',
  },

  'nord-light': {
    background: '#e5e9f0',
    foreground: '#414858',
    cursor: '#496b74',
    selectionBackground: '#d8dee9',
    black: '#3b4252',
    red: '#9a4e56',
    green: '#5c6b4e',
    yellow: '#79653e',
    blue: '#54697e',
    magenta: '#796074',
    cyan: '#496b74',
    white: '#a5abb6',
    brightBlack: '#4c566a',
    brightRed: '#bf616a',
    brightGreen: '#96b17f',
    brightYellow: '#c5a565',
    brightBlue: '#81a1c1',
    brightMagenta: '#b48ead',
    brightCyan: '#82afae',
    brightWhite: '#eceff4',
  },

  // ---- Rose Pine (Rose Pine / Rose Pine Dawn) ----

  'rose-pine': {
    background: '#191724',
    foreground: '#e0def4',
    cursor: '#e0def4',
    selectionBackground: '#403d52',
    black: '#26233a',
    red: '#eb6f92',
    green: '#538ba2',
    yellow: '#f6c177',
    blue: '#9ccfd8',
    magenta: '#c4a7e7',
    cyan: '#ebbcba',
    white: '#e0def4',
    brightBlack: '#6e6a86',
    brightRed: '#eb6f92',
    brightGreen: '#538ba2',
    brightYellow: '#f6c177',
    brightBlue: '#9ccfd8',
    brightMagenta: '#c4a7e7',
    brightCyan: '#ebbcba',
    brightWhite: '#e0def4',
  },

  'rose-pine-dawn': {
    background: '#faf4ed',
    foreground: '#575279',
    cursor: '#575279',
    selectionBackground: '#dfdad9',
    black: '#f2e9e1',
    red: '#9f586c',
    green: '#286983',
    yellow: '#926321',
    blue: '#43747d',
    magenta: '#78668d',
    cyan: '#9a5d5a',
    white: '#575279',
    brightBlack: '#9893a5',
    brightRed: '#b4637a',
    brightGreen: '#286983',
    brightYellow: '#ea9d34',
    brightBlue: '#56949f',
    brightMagenta: '#907aa9',
    brightCyan: '#d7827e',
    brightWhite: '#575279',
  },

  // ---- Tokyo Night (Night / Day) ----

  'tokyo-night': {
    background: '#1a1b26',
    foreground: '#c0caf5',
    cursor: '#c0caf5',
    selectionBackground: '#283457',
    black: '#15161e',
    red: '#f7768e',
    green: '#9ece6a',
    yellow: '#e0af68',
    blue: '#7aa2f7',
    magenta: '#bb9af7',
    cyan: '#7dcfff',
    white: '#a9b1d6',
    brightBlack: '#414868',
    brightRed: '#f7768e',
    brightGreen: '#9ece6a',
    brightYellow: '#e0af68',
    brightBlue: '#7aa2f7',
    brightMagenta: '#bb9af7',
    brightCyan: '#7dcfff',
    brightWhite: '#c0caf5',
  },

  'tokyo-night-light': {
    background: '#e1e2e7',
    foreground: '#3760bf',
    cursor: '#3760bf',
    selectionBackground: '#99a7df',
    black: '#e9e9ed',
    red: '#ba204d',
    green: '#506b34',
    yellow: '#785d35',
    blue: '#2462b7',
    magenta: '#7b44c3',
    cyan: '#006a8e',
    white: '#6172b0',
    brightBlack: '#a1a6c5',
    brightRed: '#f52a65',
    brightGreen: '#587539',
    brightYellow: '#8c6c3e',
    brightBlue: '#2e7de9',
    brightMagenta: '#9854f1',
    brightCyan: '#007197',
    brightWhite: '#3760bf',
  },

  // ---- Kanagawa (Wave / Lotus) ----

  'kanagawa-wave': {
    background: '#1f1f28',
    foreground: '#dcd7ba',
    cursor: '#dcd7ba',
    selectionBackground: '#2d4f67',
    black: '#090618',
    red: '#d0696c',
    green: '#76946a',
    yellow: '#c0a36e',
    blue: '#7e9cd8',
    magenta: '#957fb8',
    cyan: '#6a9589',
    white: '#c8c093',
    brightBlack: '#727169',
    brightRed: '#e82424',
    brightGreen: '#98bb6c',
    brightYellow: '#e6c384',
    brightBlue: '#7fb4ca',
    brightMagenta: '#938aa9',
    brightCyan: '#7aa89f',
    brightWhite: '#dcd7ba',
  },

  'kanagawa-lotus': {
    background: '#f2ecbc',
    foreground: '#545464',
    cursor: '#43436c',
    selectionBackground: '#c9cbd1',
    black: '#1f1f28',
    red: '#b73a4c',
    green: '#5a6f3f',
    yellow: '#706a3b',
    blue: '#4d699b',
    magenta: '#9e516b',
    cyan: '#4f6d68',
    white: '#545464',
    brightBlack: '#8a8980',
    brightRed: '#d7474b',
    brightGreen: '#6e915f',
    brightYellow: '#836f4a',
    brightBlue: '#6693bf',
    brightMagenta: '#624c83',
    brightCyan: '#5e857a',
    brightWhite: '#43436c',
  },

  // ---- Everforest (Dark Hard / Light Med) ----

  'everforest-dark': {
    background: '#1e2326',
    foreground: '#d3c6aa',
    cursor: '#e69875',
    selectionBackground: '#4c3743',
    black: '#7a8478',
    red: '#e67e80',
    green: '#a7c080',
    yellow: '#dbbc7f',
    blue: '#7fbbb3',
    magenta: '#d699b6',
    cyan: '#77927d',
    white: '#f2efdf',
    brightBlack: '#a6b0a0',
    brightRed: '#f85552',
    brightGreen: '#8da101',
    brightYellow: '#dfa000',
    brightBlue: '#3a94c5',
    brightMagenta: '#df69ba',
    brightCyan: '#35a77c',
    brightWhite: '#fffbef',
  },

  'everforest-light': {
    background: '#efebd4',
    foreground: '#5c6a72',
    cursor: '#f57d26',
    selectionBackground: '#eaedc8',
    black: '#7a8478',
    red: '#9b5556',
    green: '#5f6e47',
    yellow: '#7a6741',
    blue: '#4c6f6b',
    magenta: '#835e70',
    cyan: '#4c6f54',
    white: '#b2af9f',
    brightBlack: '#a6b0a0',
    brightRed: '#f85552',
    brightGreen: '#8da101',
    brightYellow: '#dfa000',
    brightBlue: '#3a94c5',
    brightMagenta: '#df69ba',
    brightCyan: '#35a77c',
    brightWhite: '#fffbef',
  },
};

export const DEFAULT_THEME = 'default-dark';
export const DEFAULT_FAMILY = 'default';

export const THEME_NAMES = Object.keys(TERMINAL_THEMES);

/** Get a theme by name, falling back to default-dark if not found. */
export function getTheme(name: string): ITheme {
  return TERMINAL_THEMES[name] ?? TERMINAL_THEMES[DEFAULT_THEME];
}

// -- WCAG contrast utilities --

/** Parse a hex color (#rrggbb or #rgb) to [r, g, b] in 0-255. */
export function parseHex(hex: string): [number, number, number] {
  const h = hex.replace('#', '');
  if (h.length === 3) {
    return [parseInt(h[0] + h[0], 16), parseInt(h[1] + h[1], 16), parseInt(h[2] + h[2], 16)];
  }
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
}

/** sRGB relative luminance per WCAG 2.1. */
export function relativeLuminance(hex: string): number {
  const [r, g, b] = parseHex(hex).map(c => {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG 2.1 contrast ratio between two hex colors (always >= 1). */
export function contrastRatio(hex1: string, hex2: string): number {
  const l1 = relativeLuminance(hex1);
  const l2 = relativeLuminance(hex2);
  const lighter = Math.max(l1, l2);
  const darker = Math.min(l1, l2);
  return (lighter + 0.05) / (darker + 0.05);
}
