# dbpinball Release Notes

| Version | Date       | Highlights |
|---------|------------|------------|

## Unreleased

_No unreleased changes yet._

---

## v0.1.7 (2026-11-15)

The first release that ships an installer, a CLI distribution path, and Safe
Sync for both PostgreSQL and MariaDB.

### Features

- **Safe Sync for PostgreSQL and MariaDB 10.11+ InnoDB.** Bounded, masked,
  verified subset synchronization from a read-only source into an enrolled local
  destination, driven by a signed plan.
  - Typed root filters: `eq`, `in`, `gte`, `lte` on integer, exact numeric, date
    and timestamp columns, with an 8-predicate cap and a keyed filter digest.
  - Parent-directed foreign-key traversal with composite-key support, canonical
    ordering, and refusal of cycles and self-references.
  - Deterministic masking with cross-table key-domain consistency, exact
    `DECIMAL(p,s)` preservation, and microsecond `DATETIME(6)` preservation.
  - Transactional import with rollback, schema-drift detection, destination
    identity binding, destination-scoped locking, `AUTO_INCREMENT`
    reconciliation, and idempotent retry.
  - Fail-closed refusals: non-InnoDB tables, triggers, unsupported types,
    non-nullable ordinary integers kept unchanged, unimplemented masking
    strategies, cross-engine plans, and policy files outside the authorized
    directory.
- **Desktop Safe Sync page** with source/destination identity inspection, plan
  review, explicit acknowledgement, execution and run status, all through the
  same core pipeline as the CLI.
- **Windows installer** (NSIS, per-user, no administrator rights) with a Start
  Menu entry, in-place upgrade and a normal uninstaller.
- **Windows CLI archive** with `install-cli.ps1`, which installs to
  `%LOCALAPPDATA%\dbpinball\bin` and adds that folder to the user `PATH`, so a
  new terminal resolves `dbpinball` with no manual editing. Uninstall removes
  only the entry it added and restores the previous `PATH` byte for byte.
- **Linux packages**: CLI tarball, `.deb` and AppImage.
- **Release provenance**: every build publishes `SHA256SUMS.txt` and
  `provenance.json` with the version, source commit, architecture and toolchain.

### Bug fixes

- The sidebar showed a hardcoded `0.1.4`; it now renders the packaged version,
  and `scripts/check-versions.cjs` fails the build when the manifests drift.
- The CLI archive could contain the desktop binary, because both crates emit
  `dbpinball.exe`; the CLI is now built into its own target directory and
  packaging refuses a binary that does not answer `--version`.
- The release CLI was quarantined by a heuristic antivirus scanner; it now
  embeds a Windows version resource and an `asInvoker` manifest.
- The desktop bundle could not be built because the Rust `tauri` crate and
  `@tauri-apps/api` were on mismatched minor versions.

### Security

- Exact `DECIMAL` precision and scale are enforced at plan and execute time; a
  narrower destination is refused as schema drift and the destination session
  is pinned to `STRICT_ALL_TABLES`.
- Privilege parsing is fail-closed: only provably read-only privileges are
  accepted, and re-grant clauses, proxy lines, role grants and unterminated
  quotes are refused.
- Masking policy files must resolve inside an authorized directory, contain no
  parent traversal, and be a regular file; the refusal never echoes the
  operating-system error.

### Known limitations

- The Windows binaries are **not code-signed**. SmartScreen warns and heuristic
  scanners can flag unsigned binaries; verify the published SHA-256 before
  running.
- macOS is not packaged.
- Non-MariaDB MySQL is refused for Safe Sync.
- `deterministic_phone`, `date_shift`, `amount_jitter` and `fixed` are declared
  in the policy schema but not implemented, and are refused.

---

## v0.1.5 (2025-11-15)

### ✨ Features
- Result grid now uses a TablePlus-style helper horizontal scrollbar:
  - Thin, always-visible bar rendered above the status bar.
  - Scrolling the helper bar scrolls the result table horizontally.
  - The table content is translated with `transform: translateX(...)` so the helper bar fully controls the viewport.
- Workspace now supports a `Ctrl+W` keyboard shortcut inside the overlay to close the active result tab, mirroring desktop editor behavior.

### ♻️ Refactors
- Streamlined the workspace top bar: replaced miscellaneous quick-action icons with a "DBpinBall" brand label and retained only essential window controls.
- Workspace overlay now automatically launches the desktop window in fullscreen for an immersive experience.

### ✅ Tests
- Added regression coverage ensuring the refined top bar renders the brand label and window controls while omitting the removed buttons.
- Added ResultGrid regression test verifying that moving the helper horizontal scrollbar updates the table's `transform` (horizontal scroll position) as expected.
- Added WorkspaceShell regression test covering the `Ctrl+W` shortcut to ensure it calls the close-tab handler for the active result tab.

---

## v0.1.4

### Notes
- Internal maintenance and styling tweaks. No major user-facing features were recorded for this release.

---

## v0.1.3

### Notes
- Internal release ahead of v0.1.4. Detailed release notes were not recorded.

---

## v0.1.2 (2025-11-11)

### ✨ Features
- Introduced a TablePlus-inspired full-screen workspace overlay triggered from the Connections page.
- Added database enumeration in the workspace sidebar by wiring new `list_databases_cmd` backend command.

### ♻️ Refactors
- Simplified the shell navigation to focus on the Connections experience with a toolbar dropdown for new connections.

### ✅ Tests
- Added Vitest + Testing Library harness with initial regression tests covering the connections workflow and workspace overlay.

---

## v0.1.0 (2025-11-11)

### ✨ Features
- Initial Tauri desktop application for Windows, macOS, and Linux.
- React + TypeScript UI with shadcn components and Tailwind CSS v4 styling.
- Connection vault with secure password storage using OS keyring.
- MySQL backups via `mysqldump` with fallback to internal sqlx dumper.
- PostgreSQL backups via `pg_dump` with sqlx fallback.
- Restore workflows with drop schema and stop-on-error options.
- Same-engine database copy (MySQL→MySQL, PostgreSQL→PostgreSQL).
- Real-time job progress via Tauri event streaming and UI log viewer.
- Job history view with status updates and artifact links.

### 🛠️ Tooling & DX
- @programinglive/commiter configured for conventional commits and semantic versioning.
- Husky commit hooks with commitlint enforcement.
- Release scripts via `npm run release`, `release:major`, `release:minor`, `release:patch`.
- Extensive documentation: README, PRD, Quick Start, Development Guide, Commiter Setup.

### 🧱 Known Limitations
- SSH tunneling not yet implemented (planned Phase 2).
- Cross-engine copy (MySQL ↔ PostgreSQL) scheduled for Phase 2.
- Job history currently stored in localStorage (future enhancement: persistent store).
- Parallel dumps, resumable jobs, and scheduled backups planned for future releases.

---

## Upcoming (Phase 2 Roadmap)
- SSH tunnel support for secure remote database access.
- Cross-engine copy with best-effort type mapping.
- Parallel table dumps and resumable jobs.
- Automated integration tests with dockerized MySQL/PostgreSQL.

For detailed requirements and roadmap, refer to [PRD.md](./PRD.md).
