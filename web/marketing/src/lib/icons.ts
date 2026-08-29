// SVG icon registry.
// Each icon: { viewBox, html, mode }
// mode "stroke" = outlined icon (stroke="currentColor", fill="none")
// mode "fill"   = filled icon (fill="currentColor", no stroke)
// mode "mixed"  = icon uses both fill and stroke inline -- wrapper sets nothing

export interface IconDef {
  viewBox: string;
  html: string;
  mode: "stroke" | "fill" | "mixed";
}

function s(d: string): string {
  return `<path stroke-linecap="round" stroke-linejoin="round" d="${d}" />`;
}

export const icons: Record<string, IconDef> = {
  github: {
    viewBox: "0 0 24 24",
    html: `<path fill="currentColor" d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z"/>`,
    mode: "fill",
  },
  download: {
    viewBox: "0 0 24 24",
    html: s("M4 16v2a2 2 0 002 2h12a2 2 0 002-2v-2M7 10l5 5 5-5M12 15V3"),
    mode: "stroke",
  },
  downloadAlt: {
    viewBox: "0 0 24 24",
    html: s("M4 16v2a2 2 0 002 2h12a2 2 0 002-2v-2M12 3v12m0 0l-4-4m4 4l4-4"),
    mode: "stroke",
  },
  copy: {
    viewBox: "0 0 24 24",
    html: `<rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/>`,
    mode: "stroke",
  },
  check: { viewBox: "0 0 24 24", html: s("M5 13l4 4L19 7"), mode: "stroke" },
  clock: {
    viewBox: "0 0 24 24",
    html: s("M12 6v6l4 2") + `<circle cx="12" cy="12" r="10"/>`,
    mode: "stroke",
  },
  plus: { viewBox: "0 0 24 24", html: s("M12 6v12m6-6H6"), mode: "stroke" },
  globe: {
    viewBox: "0 0 24 24",
    html: `<circle cx="12" cy="12" r="10"/><path d="M2 12h20M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z"/>`,
    mode: "stroke",
  },
  bidir: {
    viewBox: "0 0 24 24",
    html: s("M8 7l4-4m0 0l4 4m-4-4v18") + s("M16 17l-4 4m0 0l-4-4"),
    mode: "stroke",
  },
  externalLink: {
    viewBox: "0 0 24 24",
    html: s("M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6M15 3h6v6M10 14L21 3"),
    mode: "stroke",
  },
  menu: {
    viewBox: "0 0 24 24",
    html: s("M4 6h16M4 12h16M4 18h16"),
    mode: "stroke",
  },
  x: {
    viewBox: "0 0 24 24",
    html: s("M18 6L6 18M6 6l12 12"),
    mode: "stroke",
  },
  // Architecture diagram icons
  monitor: {
    viewBox: "0 0 24 24",
    html: `<rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/>`,
    mode: "stroke",
  },
  shield: {
    viewBox: "0 0 24 24",
    html: `<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>`,
    mode: "stroke",
  },
  "file-text": {
    viewBox: "0 0 24 24",
    html: `<path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><path d="M14 2v6h6M16 13H8M16 17H8"/>`,
    mode: "stroke",
  },
  "bar-chart": {
    viewBox: "0 0 24 24",
    html: `<path d="M18 20V10M12 20V4M6 20v-6"/>`,
    mode: "stroke",
  },
  terminal: {
    viewBox: "0 0 24 24",
    html: `<path d="M4 17l6-6-6-6M12 19h8"/>`,
    mode: "stroke",
  },
  grid: {
    viewBox: "0 0 24 24",
    html: `<rect x="2" y="2" width="20" height="20" rx="2"/><path d="M7 2v20M2 7h5M2 12h5M2 17h5"/>`,
    mode: "stroke",
  },
  layers: {
    viewBox: "0 0 24 24",
    html: `<path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5"/>`,
    mode: "stroke",
  },
  settings: {
    viewBox: "0 0 24 24",
    html: `<circle cx="12" cy="12" r="3"/><path d="M12 1v6M12 17v6M4.22 4.22l4.24 4.24M15.54 15.54l4.24 4.24M1 12h6M17 12h6M4.22 19.78l4.24-4.24M15.54 8.46l4.24-4.24"/>`,
    mode: "stroke",
  },
  play: {
    viewBox: "0 0 24 24",
    html: `<polygon points="5 3 19 12 5 21 5 3"/>`,
    mode: "stroke",
  },
  image: {
    viewBox: "0 0 24 24",
    html: `<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 9h18M9 21V9"/>`,
    mode: "stroke",
  },
};

export type IconName = keyof typeof icons;
