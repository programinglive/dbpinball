# Installation

DBPinball is built from source. There is no published installer or package
registry release documented here, and this page does not claim one.

## Clone the repository

```bash
git clone https://github.com/programinglive/dbpinball.git
cd dbpinball
```

## Build the CLI

```bash
cargo build -p dbpinball-cli --release
```

The binary is written to `target/release/dbpinball` (or `dbpinball.exe` on
Windows). Add that directory to `PATH`, or call the binary by its full path.

Verify it:

```bash
dbpinball --version
dbpinball doctor
```

## Run the desktop from source

```bash
npm install
npm run tauri dev
```

## Build desktop installers

```bash
npm run tauri build
```

Installers are written to `src-tauri/target/release/bundle/`.

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

## Where the data lives

| Purpose | Location |
| --- | --- |
| Connection profiles | `dirs::config_dir()/dbpinball/connections.json` |
| Passwords | The OS keyring, service `dbpinball`, account = profile id |

On Windows the config directory is `%APPDATA%\dbpinball`; on macOS it is
`~/Library/Application Support/dbpinball`; on Linux it is `~/.config/dbpinball`.

The CLI accepts `--config-dir <PATH>` to override the location for tests and
advanced use. The desktop always uses the default, so the two cannot drift.
