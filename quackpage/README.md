# Quakboard website

The site at https://dereckan.github.io/Quakboard/, built with Astro and Tailwind v4.

| Command | Action |
| :-- | :-- |
| `bun install` | Install dependencies |
| `bun run dev` | Dev server at `localhost:4321/Quakboard/` |
| `bun run build` | Build to `./dist/` |
| `bun run preview` | Serve the build locally |

- **Pages:** home, Download, How sync works, Support and the privacy policy, in `src/pages/`.
- **Links:** the site lives under `/Quakboard/`, so internal links go through `url()` from `src/url.ts`.
- **Downloads:** links point to `releases/latest/download/<file>` (`src/releases.ts`), so a new release needs no redeploy.
- **Colors:** the apps' palette, in `src/globals.css`; fonts are in `src/fonts/` and served from the site.
- **Deploy:** `.github/workflows/website.yml` publishes to GitHub Pages on every push to `app` that changes this folder.
