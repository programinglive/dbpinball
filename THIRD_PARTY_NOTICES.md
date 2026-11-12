# Third-Party Notices

DBPinball is distributed under the MIT License (see [`LICENSE`](LICENSE)). It
includes third-party software whose licenses require attribution when the
compiled application is distributed.

This file lists **direct** dependencies. The full transitive set is larger
(646 Rust crates and the npm tree). Before publishing a binary release, generate
a complete notice bundle with a tool such as `cargo about` or `cargo-deny`, and
include it with the installer.

## Rust runtime dependencies

| Crate | License |
| --- | --- |
| `tauri` | Apache-2.0 OR MIT |
| `tauri-plugin-opener` | Apache-2.0 OR MIT |
| `serde` | MIT OR Apache-2.0 |
| `serde_json` | MIT OR Apache-2.0 |
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

Build-time only: `tauri-build` (Apache-2.0 OR MIT).

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
- `dlopen2` and `dlopen2_derive` are MIT. The current resolution is `dlopen2 0.8.2` (declares `license = "MIT"`) and `dlopen2_derive 0.4.3`. The earlier `dlopen2 0.8.0` used a `LICENSE` file instead of an SPDX field; that file is the MIT License, Copyright (c) 2017 Szymon Wieloch.
