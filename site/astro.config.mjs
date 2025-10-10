// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
	integrations: [
		starlight({
			title: 'CAPSEM',
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/google/stcarlight' }],
			sidebar: [
				{
					label: 'Getting Started',
					items: [
						// Each item here is one entry in the navigation menu.
						{ label: 'Installation', slug: 'getting_started/installation' },
						{ label: 'Proxy Setup', slug: 'getting_started/proxy' },
					],
				},
				{
					label: 'Tutorials',
					autogenerate: { directory: 'tutorials' },
				},
				{
					label: 'Policies',
					autogenerate: { directory: 'policies' },
				},
			],
		}),
	],
});
