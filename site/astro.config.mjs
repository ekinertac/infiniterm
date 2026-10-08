// The site for infiniterm: the landing page (src/pages/index.astro) and the
// docs (Astro + Starlight), built locally and published to GitHub Pages on
// the gh-pages branch of ekinertac/infiniterm, served at infiniterm.app
// (public/CNAME names it for Pages; the DNS is on Cloudflare), so no base path.
// Guide pages are hand-written; the reference pages are generated from the
// app's own tables before every build (package.json, `reference`).
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import starlightLlmsTxt from 'starlight-llms-txt';
export default defineConfig({
  site: 'https://infiniterm.app',
  integrations: [
    starlight({
      title: 'infiniterm',
      favicon: '/favicon.png',
      // /llms.txt (llmstxt.org) plus the whole docs as llms-full.txt and a
      // trimmed llms-small.txt, built from the same pages, so an agent asked
      // to install or configure infiniterm reads one file instead of HTML.
      plugins: [
        starlightLlmsTxt({
          projectName: 'infiniterm',
          description:
            'Terminal cards on an infinite, zoomable canvas for macOS, with each coding agent\'s state (working, waiting on you, failed, done) on its card\'s border.',
          details: [
            'Native macOS app in Rust for Apple Silicon, macOS 13 or later. Free for personal use; paid work needs a licence, $29 per person, one time. Source-available, not open source.',
            '',
            'Install, non-interactively (safe for an agent to run). `--hooks` takes the agents the user actually runs, comma-separated from claude, codex, opencode, pi and cursor; `--no-hooks` wires none:',
            '',
            '```sh',
            'curl -fsSL https://infiniterm.app/install.sh | sh -s -- --hooks claude',
            '```',
            '',
            'Or `brew install --cask ekinertac/tap/infiniterm`, then `ift install-claude-hooks` (and `install-codex-hooks`, `install-opencode-hooks`, `install-pi-hooks`, `install-cursor-hooks`). `ift` is the command-line side: `ift <file>` edits a file over the current terminal card, `ift diff` opens changes against HEAD, `ift ls` lists cards, `ift commands` lists every command, `ift send`, `ift read`, `ift close` and `ift run` drive another card by its number, `ift connect user@host` opens a second window whose terminal cards run on that server over ssh, `ift licence <email> <key>` registers a commercial licence (optional).',
          ].join('\n'),
          customSets: [
            { label: 'Guide', paths: ['guide/**'], description: 'install, the canvas, card states, terminal and editor cards, ift and sessions, configuration' },
            { label: 'Reference', paths: ['reference/**'], description: 'every key, command and setting, generated from the app, and every feature in one list' },
          ],
          optionalLinks: [
            { label: 'Source', url: 'https://github.com/ekinertac/infiniterm', description: 'the code, CLAUDE.md (how it is built and why), CHANGELOG.md' },
            { label: 'Releases', url: 'https://github.com/ekinertac/infiniterm/releases/latest', description: 'signed DMG and zip; latest.json is the update feed' },
          ],
        }),
      ],
      // Link previews for the docs pages use the landing page's share image.
      head: [
        { tag: 'meta', attrs: { property: 'og:image', content: 'https://infiniterm.app/og.png' } },
        { tag: 'meta', attrs: { name: 'twitter:card', content: 'summary_large_image' } },
      ],
      description: 'Terminal cards on an infinite canvas, with the state of every coding agent visible at a glance.',
      social: [
        { icon: 'github', label: 'GitHub', href: 'https://github.com/ekinertac/infiniterm' },
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
          items: ['reference/keys', 'reference/commands', 'reference/settings', 'reference/features'],
        },
      ],
    }),
  ],
});
