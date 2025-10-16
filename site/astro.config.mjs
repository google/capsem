// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
	integrations: [
		starlight({
			title: 'CAPSEM',
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/google/capsem' }],
			sidebar: [
				{
					label: 'Getting Started',
					items: [
						// Each item here is one entry in the navigation menu.
						{ label: 'CAPSEM package installation', slug: 'getting_started/installation' },
						{ label: 'CAPSEM proxy installation', slug: 'getting_started/proxy' },
					],
				},
				{
					label: 'Integration',
					autogenerate: { directory: 'tutorials' },
				},

				{
					label: 'Policies',
					autogenerate: { directory: 'policies' },
				},
				{
					label: 'Proxy',
					autogenerate: { directory: 'proxy' },
				},
			],
		}),
	],
});
