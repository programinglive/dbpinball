# Building from source

This page is for people who want to modify DBPinball. If you only want to use
it, read [Installation](/guide/installation) instead: the installers need no
Rust, Cargo or Node.js.

## Prerequisites

| Tool | Version |
| --- | --- |
| Rust | 1.77.2+ (Tauri 2 minimum supported Rust version) |
| Node.js | 20.19+ or 22.12+ (required by Vite 7) |
| Platform build dependencies | See the [Tauri prerequisites](https://tauri.app/start/prerequisites/) |

## Clone and install

```bash
git clone https://github.com/programinglive/dbpinball.git
cd dbpinball
npm ci
```

## Run the desktop in development

```bash
npm run tauri dev
```

## Build the CLI

```bash
cargo build -p dbpinball-cli --release
```

The binary is written to `target/release/dbpinball` (`dbpinball.exe` on
Windows). It is not added to `PATH`; use the full path, or install it for your
user with the packaging script described below.

## Build the release artifacts

`scripts/package-windows.ps1` produces the same files CI publishes:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\package-windows.ps1
```

Output under `artifacts\release\v<version>\`:

| File | Contents |
| --- | --- |
| `dbpinball_<version>_x64-setup.exe` | Desktop installer (NSIS, per-user) |
| `dbpinball-cli-<version>-x64.zip` | `dbpinball.exe`, `install-cli.ps1`, `CLI-README.txt` |
| `SHA256SUMS.txt` | SHA-256 of every artifact |
| `provenance.json` | Version, commit, architecture, toolchain versions |

The script never publishes anything. Creating a tag or a GitHub release is a
separate, explicitly authorized step.

On Linux the equivalent is `npm run tauri build -- --bundles deb,appimage` plus
`cargo build -p dbpinball-cli --release`; the `Package` workflow does both.

## Verify the build

| Check | Command |
| --- | --- |
| Rust formatting | `cargo fmt --all -- --check` |
| Rust lints | `cargo clippy --workspace --all-targets -- -D warnings` |
| Rust tests | `cargo test --workspace` |
| Frontend type check | `npm run lint` |
| Frontend tests | `npm test` |
| Frontend build | `npm run build` |
| Dependency boundaries | `node scripts/check-boundaries.cjs` |

These are the same checks the CI workflow runs. See [CI checks](/development/ci).

## Windows metadata

The CLI embeds a Windows version resource and an `asInvoker` manifest from
`crates/dbpinball-cli/build.rs`, so Explorer shows the product name and version
and the CLI never requests elevation. The build script is a no-op on other
platforms.
