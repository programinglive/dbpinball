# Third-Party Notices

DBPinball is distributed under the MIT License (see [`LICENSE`](LICENSE)). It
includes third-party software whose licenses require attribution when the
compiled application is distributed.

This file summarizes the direct dependencies. The authoritative record is
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md), which is generated from the
resolved dependency graph by `scripts/generate-license-bundle.cjs` and contains
every third-party crate and package with its license and the full license texts.

Both files, together with `LICENSE`, are shipped inside the CLI archive, the
Linux tarball and the desktop installer.

## Status of published releases

- **v0.1.7** was published before the bundle existed, without a bundled notice
  file. The repository's `LICENSE`, this file and the source tree are the
  attribution record for that release. It is not retroactively replaced.
- Releases built after this change carry `LICENSE`, `THIRD_PARTY_NOTICES.md` and
  `THIRD_PARTY_LICENSES.md` inside the artifacts.

## Rust runtime dependencies

| Crate | License |
| --- | --- |
| `tauri` | Apache-2.0 OR MIT |
| `tauri-plugin-opener` | Apache-2.0 OR MIT |
| `serde` | MIT OR Apache-2.0 |
| `serde_json` | MIT OR Apache-2.0 |
| `serde_yaml` | MIT OR Apache-2.0 |
| `tokio` | MIT |
| `sqlx` | MIT OR Apache-2.0 |
| `anyhow` | MIT OR Apache-2.0 |
| `thiserror` | MIT OR Apache-2.0 |
| `chrono` | MIT OR Apache-2.0 |
| `async-compression` | MIT OR Apache-2.0 |
| `keyring` | MIT OR Apache-2.0 |
| `which` | MIT |
| `dirs` | MIT OR Apache-2.0 |
| `uuid` | Apache-2.0 OR MIT |
| `sha2` | MIT OR Apache-2.0 |
| `hmac` | MIT OR Apache-2.0 |
| `getrandom` | MIT OR Apache-2.0 |
| `unicode-normalization` | MIT OR Apache-2.0 |
| `clap` | MIT OR Apache-2.0 |
| `rpassword` | Apache-2.0 |

Development and test only, not shipped in a binary: `tempfile` (MIT OR
Apache-2.0), `url` (MIT OR Apache-2.0).

Build-time only: `tauri-build` (Apache-2.0 OR MIT), `winresource` (MIT).

## npm runtime dependencies

| Package | License |
| --- | --- |
| `@hookform/resolvers` | MIT |
| `@radix-ui/react-checkbox` | MIT |
| `@radix-ui/react-dialog` | MIT |
| `@radix-ui/react-dropdown-menu` | MIT |
| `@radix-ui/react-label` | MIT |
| `@radix-ui/react-progress` | MIT |
| `@radix-ui/react-scroll-area` | MIT |
| `@radix-ui/react-select` | MIT |
| `@radix-ui/react-slot` | MIT |
| `@radix-ui/react-tabs` | MIT |
| `@radix-ui/react-toggle` | MIT |
| `@tauri-apps/api` | Apache-2.0 OR MIT |
| `@tauri-apps/plugin-opener` | MIT OR Apache-2.0 |
| `class-variance-authority` | Apache-2.0 |
| `clsx` | MIT |
| `lucide-react` | ISC |
| `react` | MIT |
| `react-dom` | MIT |
| `react-hook-form` | MIT |
| `react-router-dom` | MIT |
| `tailwind-merge` | MIT |
| `tailwindcss-animate` | MIT |
| `zod` | MIT |

Development-only dependencies (Vitest, Testing Library, TypeScript, Vite,
PostCSS, Tailwind, commitlint, Husky, standard-version) are not part of the
distributed application, but their licenses are recorded in `package-lock.json`.

## Bundled assets

| Asset | Origin | License |
| --- | --- | --- |
| `public/logo_dbpinball.png` | Project asset | Ownership to be confirmed by the maintainer |
| `public/tauri.svg` | Tauri project | Apache-2.0 OR MIT |
| `public/vite.svg` | Vite project | MIT |

## Notes

- No dependency in the current tree uses GPL, AGPL, SSPL, or a non-commercial
  license.
- Five transitive Rust crates use MPL-2.0. MPL-2.0 is file-level copyleft and is
  compatible with distributing this application under MIT, provided the
  MPL-covered files are not modified and their notices are retained.
- `dlopen2` and `dlopen2_derive` are MIT. The current resolution is `dlopen2 0.8.2`
  (declares `license = "MIT"`) and `dlopen2_derive 0.4.3`. The earlier
  `dlopen2 0.8.0` used a `LICENSE` file instead of an SPDX field; that file is the
  MIT License, Copyright (c) 2017 Szymon Wieloch.
- The project logo's ownership is not established in the repository. It is
  recorded here rather than assumed; the maintainer must confirm it before any
  redistribution outside this project.
