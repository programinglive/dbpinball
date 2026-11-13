# CI checks

The CI workflow is `.github/workflows/ci.yml`. It runs on every pull request and
on pushes to `main`.

## Jobs

| Job | What it runs |
| --- | --- |
| Frontend (lint, test, build) | `npm run lint`, `npm test`, `npm run build` |
| Rust (fmt, clippy, test) | `cargo fmt --check`, boundary check, `cargo clippy -D warnings`, `cargo test --workspace` |
| PHP / Laravel Adapter | `composer validate`, `composer audit`, `phpunit` |
| Database integration | Core and CLI integration tests against PostgreSQL and MySQL |
| Dependency audit | `npm audit`, `cargo audit` |
| Secret scan | gitleaks |

## Permissions

Every job only reads the repository (`permissions: contents: read`). No job
deploys.

## Concurrency

`concurrency.group: ci-$&#123;&#123; github.ref &#125;&#125;` with
`cancel-in-progress: true`, so a new push cancels the previous run for the same
ref.

## Running the same checks locally

```bash
cargo fmt --all -- --check
node scripts/check-boundaries.cjs
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run lint
npm test
npm run build
```

## Related

- [Testing](/development/testing)
- [Pull request workflow](/development/pull-requests)
