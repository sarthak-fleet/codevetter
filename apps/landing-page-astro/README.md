# CodeVetter landing (Astro)

Static marketing site for [codevetter.com](https://codevetter.com). Deploys to Cloudflare Pages project `codevetter`.

## Deploy

```bash
npm run build
npx wrangler pages deploy dist --project-name=codevetter --branch=main
```

CI: `.github/workflows/deploy-landing.yml` on pushes to `apps/landing-page-astro/**`.

## Custom-domain cache

`codevetter.com` sits on a Cloudflare zone with the fleet HTML cache rule (`psi-swarm/scripts/deploy-cf-cache-rules.mjs`). After deploy, **purge the zone edge cache** or canonical URLs can serve stale HTML for up to 24h (old `s-maxage`) even while `codevetter.pages.dev` is fresh.

Post-deploy (needs `Zone.Cache Purge` on the API token):

```bash
CLOUDFLARE_API_TOKEN=... node scripts/purge-edge-cache.mjs
```

Or: Cloudflare dashboard → **codevetter.com** → Caching → **Purge Everything**.

GitHub Actions secrets:
- `CLOUDFLARE_ZONE_ID_CODEVETTER` — `c1e6464302240c22f727ce64262136fe`
- Org `CLOUDFLARE_API_TOKEN` must include **Zone.Cache Purge** (purge step currently 401 without it)

HTML cache headers (`public/_headers`): `s-maxage=300`, `stale-while-revalidate=60`.