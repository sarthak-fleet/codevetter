import { defineConfig } from 'blume';

/**
 * Blume configuration for the CodeVetter docs site.
 *
 * The committed Markdown under docs/ is the source of truth. Blume is only
 * the presentation and search layer — generated output (.blume/) is
 * gitignored and never committed.
 *
 * See docs/development/docs.md for the documentation rules.
 */
// PRDs + OpenSpec are public while this repo is open-source.
// Flip to closed-source by building with DOCS_PUBLIC_INTERNAL=false.
const publicInternal = process.env.DOCS_PUBLIC_INTERNAL !== 'false';

export default defineConfig({
  title: 'CodeVetter docs',
  description:
    'Local-first knowledge system for CodeVetter — the AI desktop code review workbench for agent-generated code.',

  content: {
    root: '../docs',
    // Render committed Markdown as the docs site. Archive is excluded from
    // the rendered site (it is preserved for git history and reachable via
    // the repo, not as canonical pages). See docs/development/docs.md.
    include: ['**/*.md'],
    // archive is never rendered; PRDs/OpenSpec render while open-source.
    exclude: publicInternal ? ['archive/**'] : ['archive/**', 'prds/**', 'openspec/**'],
  },

  theme: {
    accent: 'amber', // matches the product's warm amber accent (#d4a039)
    radius: 'md',
    mode: 'system',
    fonts: {
      display: 'space-grotesk',
      body: 'inter',
      mono: 'ibm-plex-mono',
    },
  },

  search: {
    provider: 'orama',
  },

  markdown: {
    imageZoom: true,
    code: {
      icons: true,
      wrap: false,
    },
    codeBlocks: {
      theme: {
        light: 'github-light',
        dark: 'github-dark',
      },
    },
  },

  ai: {
    llmsTxt: true,
    mcp: {
      enabled: false,
      route: '/mcp',
    },
  },

  seo: {
    og: { enabled: true },
    sitemap: true,
    robots: true,
    structuredData: true,
  },

  deployment: {
    // Served at codevetter.com/docs (apex subpath — no separate product).
    base: '/docs',
    site: 'https://codevetter.com',
    output: 'static',
  },
});
