# Developer troubleshooting

## The build fails on the boundary check

```bash
node scripts/check-boundaries.cjs
```

`dbpinball-core` must not depend on `tauri` or `clap`, directly or transitively.
Move the offending code into an adapter.

## Clippy fails with `-D warnings`

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Fix the lint rather than lowering the bar. CI runs the same command.

## The frontend type check fails

```bash
npm run lint
```

This is `tsc --noEmit`. It catches type errors that the test runner does not.

## A Rust test leaves a lock behind

The lock is released when the `StoreLock` guard drops. A test that panics while
holding it leaves `connections.lock` in the temp directory. The next test uses a
fresh temp directory, so this is harmless. In production, a dead owner is
recovered automatically.

## The cross-process test is flaky

`crates/dbpinball-cli/tests/store_concurrency.rs` spawns the real binary. It
needs the binary to be built, which `cargo test` does. If it fails, check that
the temp directory is writable and that `ps`/`tasklist` is available.

## The docs site does not build

```bash
cd website
npm install
npm run build
```

The build fails on a broken internal link. Fix the link rather than disabling the
check.

## Related

- [Testing](/development/testing)
- [CI checks](/development/ci)
