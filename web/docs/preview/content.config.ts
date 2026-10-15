import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';
import { docsSchema } from '@astrojs/starlight/schema';
import { approvedDocs } from '../approved-docs.mjs';

export const collections = {
  docs: defineCollection({
    loader: glob({
      base: './src/content/docs',
      pattern: approvedDocs,
      generateId: ({ entry }) => entry.replace(/\.mdx?$/, '').replace(/\/index$/, ''),
    }),
    schema: docsSchema(),
  }),
};
