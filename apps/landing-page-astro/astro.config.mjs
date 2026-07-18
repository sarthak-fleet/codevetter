// @ts-check
import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';
import tailwind from '@tailwindcss/vite';

// CodeVetter landing — pure static Astro.
//
// Mirrors the fleet web stack standard (../../../AGENTS.md → "Fleet web
// stack standard"):
//   - output: 'static' (no SSR adapter; this is a marketing page)
//   - build.format: 'directory' so /faq renders as `/faq/index.html`,
//     served at /faq without a 308 redirect on CF Pages Workers Assets.
//   - build.inlineStylesheets: 'always' — flat-inline per-page CSS so
//     the first paint never blocks on an external request.
//   - Lightning CSS as both transformer and minifier.
//
// Tailwind v4 via the official `@tailwindcss/vite` plugin. The single
// `globals.css` entrypoint is imported from `Layout.astro`.
export default defineConfig({
  site: 'https://codevetter.com',
  output: 'static',
  trailingSlash: 'never',
  build: {
    format: 'directory',
    inlineStylesheets: 'always',
  },
  integrations: [sitemap()],
  vite: {
    plugins: [tailwind()],
    css: { transformer: 'lightningcss' },
    build: { cssMinify: 'lightningcss' },
  },
});
