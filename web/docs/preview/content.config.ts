import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';
import { docsSchema } from '@astrojs/starlight/schema';

const sdkMetadata: Record<string, {title: string; description: string}> = {
  'sdk/python': {
    title: 'Python SDK',
    description: 'How do I install and use the Capsem Python SDK?',
  },
  'sdk/typescript': {
    title: 'TypeScript SDK',
    description: 'How do I install and use the Capsem TypeScript SDK?',
  },
};

const markdown = glob({
  base: './src/content/docs',
  pattern: [
    'getting-started/*.md',
    'architecture/security.md',
    'guides/index.md',
    'reference/index.md',
    'sdk/*.md',
  ],
  generateId: ({ entry }) => entry.replace(/\.md$/, '').replace(/\/index$/, ''),
});

export const collections = {
  docs: defineCollection({
    loader: {
      ...markdown,
      load: (context) => markdown.load({
        ...context,
        parseData: (entry) => context.parseData({
          ...entry,
          data: {...entry.data, ...sdkMetadata[entry.id]},
        }),
      }),
    },
    schema: docsSchema(),
  }),
};
