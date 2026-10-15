import { defineConfig } from 'astro/config';
import mermaid from 'astro-mermaid';
import starlight from '@astrojs/starlight';
import { unified } from '@astrojs/markdown-remark';

const links = (...slugs) => slugs.map((slug) => ({ slug }));
function localDocsLinks() {
  return (tree) => {
    function visit(node) {
      // rustdoc ignores application-context fragments; the site still highlights Rust.
      if (node.type === 'code' && node.lang === 'rust,ignore') node.lang = 'rust';
      if (node.type === 'link' && node.url?.startsWith('https://docs.capsem.org/')) {
        node.url = node.url.slice('https://docs.capsem.org'.length);
      }
      node.children?.forEach(visit);
    }
    visit(tree);
  };
}
export default defineConfig({
  site: 'https://docs.capsem.org',
  srcDir: './preview',
  redirects: {
    '/': '/getting-started/intro/',
    '/guides/writing-access-rules/': '/guides/enterprise/enforce-security-policy/',
    '/guides/writing-detection-rules/': '/guides/enterprise/write-detection-rules/',
    '/guides/sdk/': '/guides/developers/integrate-the-sdk/',
  },
  markdown: { processor: unified({ remarkPlugins: [localDocsLinks] }) },
  integrations: [
    mermaid({ mermaidConfig: { flowchart: { useMaxWidth: false } } }),
    starlight({
      title: 'Capsem',
      customCss: ['./src/styles/diagrams.css'],
      sidebar: [
        { label: 'Getting started', items: links('getting-started/intro', 'getting-started/installation', 'getting-started/how-to-use-capsem') },
        { label: 'Guides', items: [
          { slug: 'guides' },
          { label: 'Users', items: links('guides/users/use-the-terminal', 'guides/users/reuse-your-environment', 'guides/users/run-evaluations', 'guides/users/configure-security') },
          { label: 'Enterprise', items: [
            ...links('guides/enterprise/build-an-image', 'guides/enterprise/deploy-capsem', 'guides/enterprise/configure-a-vm', 'guides/enterprise/enforce-security-policy', 'guides/enterprise/write-detection-rules'),
            { label: 'Observability', items: links('guides/enterprise/investigate-a-vm', 'reference/telemetry') },
            { label: 'Contracts', collapsed: true, items: links('reference/credentials', 'reference/enforcement-rules', 'reference/sigma', 'reference/manifests') },
          ] },
          { label: 'Developers', items: links('guides/developers/integrate-the-sdk') },
        ] },
        { label: 'Architecture', items: links('architecture/overview', 'architecture/security', 'architecture/security-engine') },
        { label: 'SDK', items: links('sdk', 'sdk/python', 'sdk/typescript', 'sdk/rust', 'sdk/errors') },
        { label: 'Reference', items: links('reference', 'reference/cli', 'reference/mcp', 'reference/configuration', 'reference/credentials', 'reference/enforcement-rules', 'reference/cel', 'reference/sigma', 'reference/telemetry', 'reference/manifests') },
      ],
    }),
  ],
});
