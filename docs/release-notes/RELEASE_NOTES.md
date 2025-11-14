# dbpinball Release Notes

## Unreleased

### ✨ Features
- Result grid now uses a TablePlus-style helper horizontal scrollbar:
  - Thin, always-visible bar rendered above the status bar.
  - Scrolling the helper bar scrolls the result table horizontally.
  - The table content is translated with `transform: translateX(...)` so the helper bar fully controls the viewport.

### ♻️ Refactors
- Streamlined the workspace top bar: replaced miscellaneous quick-action icons with a "DBpinBall" brand label and retained only essential window controls.
- Workspace overlay now automatically launches the desktop window in fullscreen for an immersive experience.

### ✅ Tests
- Added regression coverage ensuring the refined top bar renders the brand label and window controls while omitting the removed buttons.
- Added ResultGrid regression test verifying that moving the helper horizontal scrollbar updates the table's `transform` (horizontal scroll position) as expected.

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
