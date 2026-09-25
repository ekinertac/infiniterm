// The documentation site for infiniterm: Astro + Starlight, built locally
// and published to GitHub Pages on the public ekinertac/infiniterm-releases
// repo, hence the base path. The landing page will live in this project too.
// Guide pages are hand-written; the reference pages are generated from the
// app's own tables before every build (package.json, `reference`).
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://ekinertac.github.io',
  base: '/infiniterm-releases',
  integrations: [
    starlight({
      title: 'infiniterm',
      favicon: '/favicon.png',
      description: 'Terminal cards on an infinite canvas, with the state of every coding agent visible at a glance.',
      social: [
        { icon: 'github', label: 'Releases', href: 'https://github.com/ekinertac/infiniterm-releases' },
      ],
      sidebar: [
        {
          label: 'Guide',
          items: [
            'guide/install',
            'guide/canvas',
            'guide/card-states',
            'guide/ift-and-sessions',
            'guide/configuration',
          ],
        },
        {
          label: 'Reference',
          items: ['reference/keys', 'reference/commands', 'reference/settings'],
        },
      ],
    }),
  ],
});
