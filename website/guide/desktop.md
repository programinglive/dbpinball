# Desktop basics

The desktop application is a Tauri shell around a React UI. It uses the same
Rust core and the same connection store as the CLI.

## The connection vault

The main screen lists your saved connections. From here you can:

- create a MySQL or PostgreSQL connection;
- edit an existing connection;
- test a connection;
- delete a connection;
- open the workspace for a connection.

## The workspace

Double-click a connection (or use the context menu) to open the workspace. It
lists databases on the server, lists tables in the selected database, and runs
SQL with results rendered in a grid.

## Safe Sync &amp; Full Copy

Open **Safe Sync** from the left sidebar to copy data from a read-only source
into an enrolled local development database (`127.0.0.1` / `localhost`,
non-production, without SSH tunnel):

- **Subset Sync (Masked):** Define a YAML or JSON root selection, generate and
  validate a dry-run plan, then execute with explicit `plan_fingerprint`
  acknowledgement.
- **Full Copy:** Copy all base tables (schema and rows) from a MySQL/MariaDB or
  PostgreSQL source into an enrolled local destination with a live progress bar,
  table counter, and row counter.

## Built-in SSH Tunnel &amp; In-App Updates

- **SSH Tunnel:** Each connection can optionally forward a local port through an
  SSH bastion using `ssh` and your local private key (for example
  `~/.ssh/id_ed25519`).
- **Check Updates:** Use the **Check Updates** button in the bottom-left sidebar
  to check GitHub Releases for a newer version and launch the installer directly.

## What the desktop stores

The desktop no longer uses browser `localStorage` as its source of truth. It
reads and writes the shared store through Tauri commands. On first launch it
migrates any profiles left in the old `localStorage` key and shows a report.

`localStorage` is kept as a read-only rollback artifact for one release. It is
never written again and never deleted.

## Migration report

After a migration the desktop shows what happened:

- how many profiles were imported;
- how many already existed;
- how many could not be read;
- how many conflicted with an existing profile.

If any record was skipped or conflicted, the report is stored and the warning
stays visible after a restart until you resolve it.

## What is not routed in the UI

The following Tauri commands exist, but their pages are not routed in the
current app:

- backup;
- restore;
- copy;
- job history.

Do not rely on them. See the [roadmap](/roadmap).

## Related

- [Managing connections](/guide/connections)
- [Credentials and keyring](/guide/credentials)
- [Connection store](/architecture/connection-store)
