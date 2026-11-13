# DBPinball documentation site

The official documentation for DBPinball, built with
[VitePress](https://vitepress.dev/) and deployed to Cloudflare Pages at
<https://dbpinball.programinglive.com>.

## Local development

```bash
cd website
npm install
npm run dev
```

The dev server prints a local URL. Content is written in Markdown under
`website/`.

## Build

```bash
cd website
npm run build
```

The static output is written to `website/.vitepress/dist`.

## Link check

```bash
cd website
node scripts/check-links.mjs
```

The check reads the built output and fails on an internal link that does not
resolve. It is offline and deterministic.

## Deployment

| Item | Value |
| --- | --- |
| Host | Cloudflare Pages |
| Project | `dbpinball-docs` |
| Production URL | <https://dbpinball.programinglive.com> |
| Pages URL | <https://dbpinball-docs.pages.dev> |
| Workflow | `.github/workflows/docs.yml` |
| Production branch | `main` |

The workflow builds and link-checks on every pull request, and deploys only on a
push to `main`. A pull request never deploys.

Required repository secrets:

| Secret | Purpose |
| --- | --- |
| `CLOUDFLARE_API_TOKEN` | Pages deploy token |
| `CLOUDFLARE_ACCOUNT_ID` | Target account |

No secret is printed in the workflow logs.

## Rollback

Cloudflare Pages keeps every deployment. To roll back:

1. Open the Cloudflare dashboard → Workers & Pages → `dbpinball-docs` →
   Deployments.
2. Find the last known-good production deployment.
3. Use **Rollback to this deployment**.

Command-line alternative:

```bash
npx wrangler pages deployment list --project-name dbpinball-docs
npx wrangler pages deployment rollback <deployment-id> --project-name dbpinball-docs
```

A code-level rollback is also available: revert the offending commit on `main`
and let the workflow deploy the reverted build. Do not rewrite `main` history.

## DNS

| Record | Type | Content | Proxy |
| --- | --- | --- | --- |
| `dbpinball.programinglive.com` | CNAME | `dbpinball-docs.pages.dev` | Proxied |

The custom domain is also registered on the Pages project. TLS is provisioned by
Cloudflare.

## Adding a page

1. Create a Markdown file under `website/`.
2. Add it to the sidebar in `website/.vitepress/config.mts`.
3. Run `npm run build` and `node scripts/check-links.mjs`.
4. Do not present planned functionality as released. Use the status labels in
   `roadmap.md`.