# Pull request workflow

## 1. Branch

Create a branch from `main`:

```bash
git checkout -b feat/<short-description>
```

## 2. Implement and test

Run the same checks CI runs:

```bash
cargo fmt --all -- --check
node scripts/check-boundaries.cjs
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run lint
npm test
npm run build
```

## 3. Open the pull request

- Target `main`.
- Describe the problem, the change and the verification.
- Link the issue if there is one.

## 4. Review

CI must be green. A reviewer checks:

- the change stays inside its layer;
- the core is still headless;
- no credential path was added;
- the tests actually prove the behaviour.

## 5. Merge

The repository uses merge commits. Do not force-push a branch under review.

## Documentation changes

Documentation lives in `website/`. A docs change must build:

```bash
cd website
npm run build
```

## Related

- [CI checks](/development/ci)
- [Contributing](/development/contributing)
