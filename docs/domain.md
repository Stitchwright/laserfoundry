# The `laserfoundry.app` domain

The hosted browser build is at `https://stitchwright.github.io/laserfoundry/` (GitHub Pages, built by
`.github/workflows/pages.yml`). This is how to serve it from `https://laserfoundry.app/` as well. Nothing in the
program depends on the address: the page uses relative paths, so it works at either.

`.app` domains are on the browsers' HTTPS-only list, which suits this page (its content security policy already
requires HTTPS for everything it talks to).

## 1. Register (Cloudflare Registrar)

Sold at the registry's cost, with WHOIS privacy and DNS included. About $8.20 the first year and $14.20 a year
after (a third-party price tracker, 2026-10-13: check the checkout). Turn on auto-renew. The registrar account
is the key to the domain: use a strong password and two-factor sign-in.

## 2. DNS records (Cloudflare → the domain → DNS → Records)

Every record **DNS only** (the grey cloud, not proxied): GitHub has to see the name itself to issue the HTTPS
certificate, and proxying can stall that. The proxy can be turned on later if wanted.

| Type  | Name | Content                  |
|-------|------|--------------------------|
| A     | @    | 185.199.108.153          |
| A     | @    | 185.199.109.153          |
| A     | @    | 185.199.110.153          |
| A     | @    | 185.199.111.153          |
| AAAA  | @    | 2606:50c0:8000::153      |
| AAAA  | @    | 2606:50c0:8001::153      |
| AAAA  | @    | 2606:50c0:8002::153      |
| AAAA  | @    | 2606:50c0:8003::153      |
| CNAME | www  | stitchwright.github.io   |

(These are GitHub Pages' published addresses; check GitHub's "Managing a custom domain for your GitHub Pages
site" page before adding them, in case they have changed.)

## 3. Verify the domain with GitHub (stops anyone else claiming it)

Organisation settings → Pages → Add a domain → `laserfoundry.app`. GitHub shows a `_github-pages-challenge-…`
**TXT** record name and value: add it in Cloudflare, then press Verify. Do this *before* step 4.

## 4. Tell the repository

Repository Settings → Pages → Custom domain → `laserfoundry.app` → Save, wait for the certificate (a few
minutes, up to an hour), then tick **Enforce HTTPS**. `www.laserfoundry.app` then redirects to the apex.

## 5. Then, in the repository (ask the agent to do this once the above works)

- Point the README, the docs and the repository's website field at `https://laserfoundry.app/`.
- The old `stitchwright.github.io/lightfoundry` address may stop working after the repository rename; use the new one.
- The apps use the shared Foundry service (`https://cloud.stitchfoundry.app`). If it gets a name such as
  `api.laserfoundry.app`, set the `LF_CLOUD_URL` repository variable to it (see `ai.md`): the browser build's content security policy is generated from that variable.

## Checking it

`dig +short laserfoundry.app` lists the four addresses; `curl -sI https://laserfoundry.app/` answers `200` with a
certificate for `laserfoundry.app`; opening the page shows the app.
