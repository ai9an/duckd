# AGENTS.md — duckd website

Static landing/download page for [duckd](https://github.com/ai9an/duckd), living inside the same repo under `/website`. Read this fully before writing anything.

## What this is

A single-page site at `duckd.ai9an.com`. Visitors land, see the terminal-style demo, and get a download button matching their OS, with smaller links for the other platform underneath. No backend, no build step required — plain HTML/CSS/JS.

## Stack

Vanilla HTML/CSS/JS. No framework, no bundler. This is a one-pager, adding Vite/React etc. would be dead weight for what's a handful of DOM elements and one animation. Keep it dependency-free.

## Where it lives in the repo

The repo root is already the Tauri app's frontend (`index.html`, `src/`, `vite.config.ts` etc. — don't touch those). The website is fully separate, under `/website`:

```
duckd/
  website/
    AGENTS.md              # this file — save it here, nested AGENTS.md files scope to their subtree
    index.html
    style.css
    script.js
    previews/
      app-preview.png       # already in repo at /preview/app-preview.png — copy or symlink in
      ...                   # more screenshots will be added here by the user before/during build
    CNAME                   # contents: duckd.ai9an.com
  .github/
    workflows/
      deploy-website.yml
```

## Deployment

GitHub Pages' branch-based "deploy from /docs" mode won't work here since the site lives in `/website`, not `/docs`, and the repo root is already spoken for by the app. Use a GitHub Actions deployment instead:

- Repo Settings → Pages → Build and deployment → Source: **GitHub Actions**.
- Workflow uploads `./website` as the Pages artifact and deploys it. Standard pattern:

```yaml
name: Deploy website
on:
  push:
    branches: [main]
    paths: ["website/**"]
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: "pages"
  cancel-in-progress: true

jobs:
  deploy:
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/upload-pages-artifact@v3
        with:
          path: ./website
      - id: deployment
        uses: actions/deploy-pages@v4
```

The `paths: ["website/**"]` filter matters — without it, every app commit would redeploy the site.

- `CNAME` must sit inside `/website` (the folder being uploaded), not the repo root, or the custom domain won't take.
- DNS: a CNAME record for `duckd` pointing at `ai9an.github.io` needs to exist on the domain's Cloudflare zone — same pattern as the other subdomains under [[domain-projects]]. This is a manual DNS step, not something the workflow does.

## OS detection + downloads

Detect via `navigator.userAgent` (don't rely on `navigator.platform`, it's deprecated). Windows vs everything-else is enough — this app only ships Windows and Linux builds, and Linux can't really be sub-detected reliably from the UA string anyway, so: Windows → show the `.exe`/`.msi` as primary; anything else → show the AppImage as primary. Show the non-primary one as a smaller secondary link either way, don't hide it.

Downloads are **static links, manually updated per release** — no API calls. Keep every link in one small object at the top of `script.js` so a release update is a one-line-per-platform edit, not a hunt through the HTML:

```js
const DOWNLOADS = {
  windows: {
    label: "Download for Windows",
    url: "https://github.com/ai9an/duckd/releases/download/v1/duckd_1.0.0_x64-setup.exe",
  },
  linux: {
    label: "Download for Linux (AppImage)",
    url: "https://github.com/ai9an/duckd/releases/download/v1/duckd_1.0.0_amd64.AppImage",
  },
};
```

Pull the actual current asset URLs from the [v1 release page](https://github.com/ai9an/duckd/releases/tag/v1) rather than guessing filenames.

One suggestion worth considering, not a requirement: if future CI builds keep asset filenames version-free (e.g. `duckd-setup.exe` instead of `duckd_1.2.0_x64-setup.exe`), the static links only need updating when something actually changes about the build, not on every version bump. Worth raising with the user rather than deciding unilaterally, since it touches the app's release workflow, not just the site.

## Hero: animated terminal demo

The centerpiece. A mocked-up terminal/HUD block (not a real shell, just styled div/pre) that plays a short typing animation on load and loops: something like a hotkey being "pressed" and a preset applying, e.g.

```
$ Alt+3 pressed → preset "lockin"
  discord     100% → 20%
  spotify      80% →  0%
  firefox-bin  60% →  0%
```

Implement as a simple JS typewriter (setTimeout/setInterval revealing characters), CSS blinking cursor at the end of the active line. Keep it readable at a glance — this is a 2-3 second loop, not a novel. Respect `prefers-reduced-motion`: fall back to the fully-typed static text with no animation for users with that set.

## Visual direction

Match the app itself, don't invent a new palette:
- Same near-black background (`#0a0a0c`–`#121214` range) as the app UI.
- Same monospace font (JetBrains Mono or Fira Code — whichever the app ended up using, keep them consistent).
- One accent color, used for the primary download button, the terminal cursor, and links — nothing else competes with it.
- Layout: hero (title, tagline, terminal demo, download button) → feature list (pull from the README's feature bullets) → screenshot gallery → footer with GitHub link.

## Screenshots

Gallery pulls from `/website/previews/`. Only `app-preview.png` exists there right now; more are coming from the user directly into that folder. Build the gallery to be trivially extendable — a simple grid that just needs another `<img>` tag or array entry per screenshot, not a data pipeline. Don't block the rest of the build on having more than one image.

## Non-goals

- No dark/light theme toggle — the site is dark-only, matching the app.
- No analytics/tracking scripts.
- No dynamic release-fetching (explicitly decided against for this version).
