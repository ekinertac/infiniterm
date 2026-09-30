// The site for infiniterm: the landing page (src/pages/index.astro) and the
// docs (Astro + Starlight), built locally and published to GitHub Pages on
// the public ekinertac/infiniterm-releases repo, served at infiniterm.app
// (public/CNAME names it for Pages; the DNS is on Cloudflare), so no base path.
// Guide pages are hand-written; the reference pages are generated from the
// app's own tables before every build (package.json, `reference`).
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://infiniterm.app',
  integrations: [
    starlight({
      title: 'infiniterm',
      favicon: '/favicon.png',
      // Link previews for the docs pages use the landing page's share image.
      head: [
        { tag: 'meta', attrs: { property: 'og:image', content: 'https://infiniterm.app/og.png' } },
        { tag: 'meta', attrs: { name: 'twitter:card', content: 'summary_large_image' } },
      ],
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
            'guide/terminal',
            'guide/editor',
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
