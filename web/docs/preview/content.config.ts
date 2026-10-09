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
        'architecture/security-engine.md',
        'guides/index.md',
        'guides/writing-access-rules.md',
        'guides/writing-detection-rules.md',
        'guides/sdk/index.md',
        'reference/index.md',
        'reference/cel.md',
        'reference/sigma.md',
        'sdk/index.mdx',
      ],
      generateId: ({ entry }) => entry.replace(/\.mdx?$/, '').replace(/\/index$/, ''),
    }),
    schema: docsSchema(),
  }),
};
