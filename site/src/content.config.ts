// Starlight's docs collection: every .md under src/content/docs is a page.
// The reference pages there are generated (see .gitignore and package.json).
import { defineCollection } from 'astro:content';
import { docsLoader } from '@astrojs/starlight/loaders';
import { docsSchema } from '@astrojs/starlight/schema';

export const collections = {
  docs: defineCollection({ loader: docsLoader(), schema: docsSchema() }),
};
