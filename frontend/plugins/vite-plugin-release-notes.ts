// @ts-nocheck
// Vite plugin: embeds the latest release notes from site/src/pages/news/ at compile time.
// Exports a virtual module `virtual:release-notes` with { html, version }.
// Type-checking disabled: this runs at build time in Vite's Node context, not in the app.

import { readFileSync, readdirSync } from 'node:fs';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { marked } from 'marked';

const VIRTUAL_ID = 'virtual:release-notes';
const RESOLVED_ID = '\0' + VIRTUAL_ID;
const __dirname = dirname(fileURLToPath(import.meta.url));

function stripFrontmatter(md) {
  if (!md.startsWith('---')) return md;
  const end = md.indexOf('---', 3);
  if (end === -1) return md;
  return md.slice(end + 3).trim();
}

function findLatestNewsFile(newsDir) {
  let files;
  try {
    files = readdirSync(newsDir).filter((f) => /^\d+\.\d+\.md$/.test(f));
  } catch {
    return null;
  }
  if (files.length === 0) return null;

  // Sort by version descending (e.g., 0.9.md > 0.8.md)
  files.sort((a, b) => {
    const va = a.replace('.md', '').split('.').map(Number);
    const vb = b.replace('.md', '').split('.').map(Number);
    for (let i = 0; i < Math.max(va.length, vb.length); i++) {
      const diff = (vb[i] ?? 0) - (va[i] ?? 0);
      if (diff !== 0) return diff;
    }
    return 0;
  });

  return { path: join(newsDir, files[0]), version: files[0].replace('.md', '') };
}

export default function releaseNotesPlugin() {
  return {
    name: 'release-notes',
    resolveId(id) {
      if (id === VIRTUAL_ID) return RESOLVED_ID;
    },
    load(id) {
      if (id !== RESOLVED_ID) return;

      const newsDir = resolve(__dirname, '../../site/src/pages/news');
      const latest = findLatestNewsFile(newsDir);
      if (!latest) {
        return `export const html = '<p>No release notes available.</p>';\nexport const version = '';`;
      }

      const raw = readFileSync(latest.path, 'utf-8');
      const md = stripFrontmatter(raw);
      const html = marked.parse(md, { async: false });

      // Escape backticks and backslashes for template literal
      const escaped = html.replace(/\\/g, '\\\\').replace(/`/g, '\\`').replace(/\$/g, '\\$');

      return `export const html = \`${escaped}\`;\nexport const version = '${latest.version}';`;
    },
  };
}
