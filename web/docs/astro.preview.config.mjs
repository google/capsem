import { defineConfig } from 'astro/config';
import mermaid from 'astro-mermaid';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://docs.capsem.org',
  srcDir: './preview',
  redirects: { '/': '/getting-started/installation/' },
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
        {
          label: 'Architecture',
          items: [{ slug: 'architecture/security' }, { slug: 'architecture/security-engine' }],
        },
        {
          label: 'Guides',
          items: [
            { slug: 'guides' },
            { slug: 'guides/writing-access-rules' },
            { slug: 'guides/writing-detection-rules' },
            { slug: 'guides/sdk' },
          ],
        },
        {
          label: 'Reference',
          items: [{ slug: 'reference' }, { slug: 'reference/cel' }, { slug: 'reference/sigma' }],
        },
        { slug: 'sdk' },
      ],
    }),
  ],
});
