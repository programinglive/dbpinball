# Contributing to DBPinball

Thanks for taking the time to contribute. This guide covers setup, the rules that keep the project consistent, and the expectations for pull requests.

## Before you start

- Read [`README.md`](README.md) for the current capability state. Several features are documented as in development or planned; do not assume they work.
- Read [`SECURITY.md`](SECURITY.md) if your change touches credentials, connections, or data handling.
- For anything larger than a small fix, open an issue first so the approach can be agreed before you write code.

## Local development setup

### Prerequisites

- Node.js 20.19+ or 22.12+
- Rust 1.77.2+
- A MySQL or PostgreSQL server for manual testing
- Tauri platform prerequisites: <https://tauri.app/start/prerequisites/>

### Install and run

```bash
npm install
npm run tauri dev
```

`npm run dev` starts the frontend alone in a browser. Tauri commands will not be available there.

## Repository structure

```text
src/                      React + TypeScript frontend
  pages/                  connections, backup, restore, copy, jobs
  components/workspace/   workspace shell, result grid, pickers
  components/ui/          shadcn/ui primitives
  hooks/                  workspace query and window hooks
src-tauri/                Rust backend
  src/lib.rs              Tauri command surface
  src/models.rs           shared data models
  src/secrets.rs          OS keyring access
  src/runners/            MySQL and PostgreSQL implementations
docs/                     architecture, audit, and release notes
scripts/                  pre-commit test verification
```

## Frontend workflow

```bash
npm run dev        # Vite dev server
npm test           # Vitest, single run
npm run test:watch # Vitest, watch mode
npm run lint       # tsc --noEmit
```

Tests live next to the code in `__tests__` directories. Add a test for any behaviour change in a component or hook.

## Rust workflow

The repository is a Cargo workspace with two members: `crates/dbpinball-core` (headless services) and `src-tauri` (the desktop adapter). Run commands from the repository root.

```bash
cargo check --workspace
cargo test --workspace
cargo test -p dbpinball-cli
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
node scripts/check-boundaries.cjs
cargo run -p dbpinball-cli -- --help
```

`dbpinball-core` must not depend on Tauri or a CLI parser; the boundary check fails the build if it does.

Database integration tests are gated and need a disposable stack:

```bash
docker compose -f docker-compose.test.yml up -d --wait
DBPINBALL_TEST_PG_URL=postgres://postgres:dbpinball@127.0.0.1:55432/dbpinball_test \
DBPINBALL_TEST_MYSQL_URL=mysql://root:dbpinball@127.0.0.1:53306/dbpinball_test \
  cargo test -p dbpinball-core --features integration
DBPINBALL_TEST_PG_URL=postgres://postgres:dbpinball@127.0.0.1:55432/dbpinball_test \
DBPINBALL_TEST_MYSQL_URL=mysql://root:dbpinball@127.0.0.1:53306/dbpinball_test \
  cargo test -p dbpinball-cli --features integration
docker compose -f docker-compose.test.yml down -v
```

New Rust code should come with unit tests; do not add code that cannot be tested.

## Formatting and linting

- Rust: `cargo fmt` and `cargo clippy`.
- TypeScript: `npm run lint` (type check only).

There is no Prettier configuration in this repository. Do not add a formatter that reformats the whole codebase in an unrelated pull request.

## Commit messages

Conventional Commits are enforced by `commitlint` through a Husky `commit-msg` hook.

```text
type(scope): subject
```

Allowed types: `feat`, `fix`, `perf`, `refactor`, `docs`, `style`, `test`, `build`, `ci`, `chore`, `revert`.

Examples:

```text
feat(workspace): add table tab close shortcut
fix(restore): handle missing psql client
docs(readme): correct capability states
```

The `pre-commit` hook runs `node scripts/verify-tests.cjs` when that script is present, and falls back to `npm test` otherwise. A fresh clone works either way.

## Continuous integration

Every push and pull request runs `.github/workflows/ci.yml`:

- frontend: `npm ci`, `npm run lint`, `npm test`, `npm run build`
- Rust: `cargo fmt --all -- --check`, `node scripts/check-boundaries.cjs`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- dependencies: `npm audit --audit-level=high`, `cargo audit`
- secrets: gitleaks

Run the same checks locally before pushing:

```bash
npm run lint && npm test && npm run build
cargo fmt --all -- --check && node scripts/check-boundaries.cjs && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
npm audit --audit-level=high
cargo audit
```

`cargo audit` reads `src-tauri/.cargo/audit.toml`. Do not add an ignore entry without a written reachability argument.

## Pull request expectations

- Keep the change focused. One concern per pull request.
- Describe what changed and why, not just what files were touched.
- Include tests for behaviour changes.
- Update documentation when behaviour, commands, or capability states change.
- Do not include unrelated formatting or refactoring.
- Do not commit generated artifacts, build output, or database dumps.

## Security-sensitive contributions

Changes to any of the following need extra care and a clear description of the threat being addressed:

- Credential storage, keyring access, or password handling.
- Connection URL construction.
- SQL execution paths.
- Anything that writes to a database.
- The planned Safe Sync pipeline, masking, or destination verification.

Rules:

- Never log a password, token, or full connection URL.
- Never add a CLI flag that accepts a password as an argument.
- Never weaken a safety check to make a test pass.
- If you add a security control, add the test that proves it works.

## Database adapter requirements

A new engine adapter must:

- Implement the same operations as the existing MySQL and PostgreSQL adapters: test connection, list databases, list tables, run query.
- Report errors with enough context to diagnose the failure without exposing credentials.
- Use the shared credential provider rather than reading secrets directly.
- Include integration tests against a disposable database.
- Document any engine-specific limitation in the adapter's module documentation.

## Documentation requirements

- `README.md` must reflect only what is implemented. Move a capability between "Available", "In development", and "Planned" when its state changes.
- Do not describe a planned feature as available.
- Do not describe raw backup, restore, or copy as sanitized or safe to share.
- Keep commands in the documentation runnable. If a command does not work, fix it or remove it.

## Backward compatibility

The desktop app is the compatibility contract:

- Existing Tauri command names and payload shapes must keep working.
- Connection data stored by a previous version must still load.
- A change that breaks either needs a migration path and a note in the pull request.

## Test data rules

**Never use real production data.**

- Use synthetic datasets.
- Use disposable local databases or containers.
- Do not attach dumps, snapshots, or connection strings to issues or pull requests.
- Do not commit fixtures that contain personal data, even masked, unless the masking is verifiably irreversible and reviewed.

## Reporting bugs

Use the issue templates. Include the version, the engine and version, and reproduction steps with synthetic data. Redact hosts, usernames, and any connection details.

## Code of conduct

Participation is governed by [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).
