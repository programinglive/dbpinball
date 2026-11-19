# Changelog

All notable changes to DBPinball are documented here.

## v0.1.8 (2026-11-19)

### Features
- **Full Copy (`safe-sync full-copy`):** Copy all base tables (schema and rows) from a read-only MySQL/MariaDB or PostgreSQL source to an enrolled local development destination without writing a YAML selection spec.
- **Live Full Copy Progress Bar:** Real-time progress events (`safesync-full-copy-progress`) showing the active table, completed table count, copied row count, and percentage.
- **In-App Updater:** Added a **Check Updates** button in the desktop sidebar to check GitHub Releases and download/launch the latest installer directly.
- **Built-in OpenSSH Tunnel Support:** Added SSH bastion forwarding (`--ssh-tunnel`, `--ssh-host`, `--ssh-port`, `--ssh-user`, `--ssh-key`, `--ssh-strict-host-key`) across `dbpinball-core`, `dbpinball-cli`, and the Desktop connection dialog.

### Improvements & Bug Fixes
- **Safe Sync Fullscreen Layout:** Revamped the Safe Sync page into a full-width responsive 2-column layout with compact identity summary cards and collapsible raw JSON.
- **Connection Dialog Polish:** Fixed sticky header/footer layout, scrollable form body, dark custom scrollbars, non-wrapping SSH tunnel toggle, and added a **Target** selector (`local`, `staging`, `production`, `unknown`).
- **Streaming & Reliability:** Switched Full Copy to single-query `sqlx` streaming with 1,000-row batches, per-table transactions, proper MariaDB `BIT` (`b'...'`) and binary hex literal formatting, and cleared source statement timeouts for large tables.
- **Safety Guard:** Restricted Safe Sync and Full Copy destinations in the Desktop UI to non-production localhost connections without SSH tunnels.

## v0.1.7

### Features & Improvements
- **Safe Sync & Destination Enrollment:** Added `safe-sync` (`inspect`, `plan`, `validate`, `execute`, `status`, `key-rotate`) and `security` (`enroll`, `status`, `forget`, `policy-check`) commands with OS keyring integration.
- **TablePlus-Inspired Desktop UI:** Polished workspace sidebar, connections list, and Tailwind v4 theme tokens.

## v0.1.6 (2025-11-14)

### Features
- Added dual column resizing and helper horizontal scrollbar for `ResultGrid`.
- Added `Ctrl+W` tab close shortcut in the workspace.
- Compact centered connection dialog and `sqlx` 0.8 upgrade.
