# Contributing

## Before you start

- Read the [architecture](/architecture/) so you know which layer your change
  belongs to.
- Keep the core headless. If your change needs Tauri or Clap, it belongs in an
  adapter.
- Do not add a plaintext credential path.

## Ground rules

1. **The core stays headless.** `node scripts/check-boundaries.cjs` must pass.
2. **No password in the store.** A test asserts the key is absent.
3. **No silent fallback.** If the keyring is unavailable, fail; do not write a
   plaintext file.
4. **Document what is not implemented.** Do not present planned features as
   released.
5. **Tests come with the change.** A behaviour change without a test is not
   finished.

## Commit messages

Use conventional commits:

```text
feat(core): add ...
fix(store): ...
docs: ...
test: ...
```

## Related

- [Testing](/development/testing)
- [Pull request workflow](/development/pull-requests)
