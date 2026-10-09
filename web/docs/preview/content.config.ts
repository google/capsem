import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';
import { docsSchema } from '@astrojs/starlight/schema';

export const collections = {
  docs: defineCollection({
    loader: glob({
      base: './src/content/docs',
      pattern: [
        'getting-started/*.md',
        'architecture/security.md',
        'guides/index.md',
        'guides/sdk/index.md',
        'reference/index.md',
        'sdk/index.mdx',
      ],
      generateId: ({ entry }) => entry.replace(/\.mdx?$/, '').replace(/\/index$/, ''),
    }),
    schema: docsSchema(),
  }),
};
