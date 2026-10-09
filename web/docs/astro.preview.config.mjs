import { defineConfig } from 'astro/config';
import mermaid from 'astro-mermaid';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://docs.capsem.org',
  srcDir: './preview',
  integrations: [
    mermaid(),
    starlight({
      title: 'Capsem',
      sidebar: [
        {
          label: 'Getting started',
          items: [
            { slug: 'getting-started/intro' },
            { slug: 'getting-started/installation' },
            { slug: 'getting-started/how-to-use-capsem' },
          ],
        },
        { label: 'Architecture', items: [{ slug: 'architecture/security' }] },
        {
          label: 'Guides',
          items: [{ slug: 'guides' }, { slug: 'guides/sdk' }],
        },
        { slug: 'reference' },
        { slug: 'sdk' },
      ],
    }),
  ],
});
