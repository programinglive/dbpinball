# Open Source Readiness Audit — DBPinball

Date: 2026-11-12
Repository: `D:\code\dbpinball` (origin `https://github.com/programinglive/dbpinball`)
Branch: `main` at `82a6a52`, working tree dirty (see §10)
Auditor: automated repository inspection plus manual source review

This report records what was inspected, what was found, what was changed, and what remains open. It does not claim the repository is secure because a scan found no secrets.

---

## 1. Method

| Check | How it was run |
| --- | --- |
| Tracked file inventory | `git ls-files` (105 files) |
| Working-tree state | `git status --short`, `git diff` |
| Secret patterns in tracked files | `git grep -I -n -E` over 13 patterns (private keys, cloud keys, GitHub tokens, Slack tokens, Google API keys, DB URLs, password/api_key/secret assignments) |
| Secret patterns in history | `git log --all -p` piped through the same patterns |
| Sensitive artifacts | filesystem scan for `.env`, `.dump`, `.sql`, `.pem`, `.key`, `id_rsa` outside `node_modules`/`target`/`.git` |
| Absolute paths | `git grep` for `D:\code`, `C:\Users`, `/Users/<name>`, `/home/<name>` |
| Internal infrastructure | `git grep` for internal hostnames, Tailscale ranges, private IP ranges |
| License metadata | `LICENSE` presence, `package.json`, `src-tauri/Cargo.toml`, `cargo metadata` |
| Dependency licenses | `cargo metadata` (646 packages), `node_modules/*/package.json` |
| Vulnerabilities | `npm audit --json` |
| Documentation accuracy | manual read of README, QUICKSTART, DEVELOPMENT, IMPLEMENTATION_SUMMARY, GITHUB_SETUP_SUMMARY, COMMITER_SETUP, PRD, ARCHITECTURE, and source |

Tools not available in this environment: `cargo-audit`, `cargo-license`, `gitleaks`, `trufflehog`, `markdown-link-check`. Their absence is a documented limitation, not a clean result. A custom Node script was used for relative-link checking instead.

---

## 2. Secret and credential scan

### 2.1 Result

**No live secret was found** in tracked files or in the full commit history.

- Private keys (`BEGIN … PRIVATE KEY`): 0
- Cloud access keys (`AKIA…`): 0
- GitHub tokens (`ghp_`, `github_pat_`): 0
- Slack tokens (`xox…`): 0
- Google API keys (`AIza…`): 0
- Password / API key / secret assignments with literal values: 0
- `.env`, `.pem`, `.key`, `id_rsa`, `.dump`, `.sql` files: 0 tracked, 0 untracked
- `src-tauri/target/`, `node_modules/`, `dist/`: not tracked

### 2.2 Validated non-findings

| Pattern | Hits | Verdict |
| --- | --- | --- |
| `postgresql://…:…@` | `src-tauri/src/runners/postgres.rs:21,37,172,309` | **Not a leaked secret.** These are `format!` templates with `{}` placeholders. They are a code-level risk (see F8), not a committed credential. |
| `mysql://…:…@` | `src-tauri/src/runners/mysql.rs:22,38,189,326` | Same as above. |
| Private IP ranges | `package-lock.json` (14 lines) | Registry URLs and integrity hashes, not infrastructure. |
| `10.` / `172.` matches | `package.json:55,62` | Version strings (`@testing-library/dom ^10.4.2`), not addresses. |

### 2.3 Sensitive data

No production dumps, customer records, or real connection strings are tracked. The only database-shaped strings are the URL templates above.

---

## 3. Internal infrastructure and machine paths

- Internal hostnames, Tailscale addresses, and private endpoints: **none** in tracked files. The `internal` matches in `PRD.md`, `README.md`, and `src-tauri/ARCHITECTURE.md` are the English word "internal" (internal dumper), not infrastructure.
- Machine-specific absolute paths: **one**, `QUICKSTART.md:50` (`/Users/you/dbpinball/backups/...`). It is a generic placeholder, not a real path, but it was replaced with `~/dbpinball/backups/...` for consistency.

---

## 4. License audit

### 4.1 Findings

| Item | State before | Verdict |
| --- | --- | --- |
| Root `LICENSE` | **Missing** | F1 — README claimed MIT with no license file |
| `package.json` `license` | **Missing** | F9 |
| `src-tauri/Cargo.toml` `license` | **Missing** | F9 |
| `src-tauri/Cargo.toml` `authors` | `["you"]` (template placeholder) | F9 |
| `src-tauri/Cargo.toml` `description` | `"A Tauri App"` (template placeholder) | F9 |
| `composer.json` / Laravel adapter | **Not present in this repository** | No action; see §9 |
| Source-file license headers | None | Acceptable for MIT; not required |
| Bundled assets | `public/logo_dbpinball.png`, `public/tauri.svg`, `public/vite.svg` | Provenance to confirm (F20) |

### 4.2 Dependency licenses

Rust (646 packages via `cargo metadata`): all permissive or weak-copyleft-compatible.

| License expression | Packages |
| --- | --- |
| MIT OR Apache-2.0 | 290 |
| MIT | 161 |
| Apache-2.0 OR MIT | 60 |
| MIT/Apache-2.0 | 39 |
| Zlib OR Apache-2.0 OR MIT | 18 |
| Unicode-3.0 | 18 |
| Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | 11 |
| MPL-2.0 | 5 |
| Unlicense OR MIT | 4 |
| Apache-2.0 | 4 |
| BSD-3-Clause | 4 |
| other permissive combinations | remainder |

No GPL, AGPL, SSPL, or non-commercial license was found. MPL-2.0 (5 packages) is file-level copyleft and remains compatible with distributing the application under MIT, provided the MPL-covered files are not modified and their notices are retained.

npm (direct dependencies): 37 MIT, 2 Apache-2.0, 2 Apache-2.0 OR MIT, 2 ISC, 1 MIT OR Apache-2.0. No non-permissive license.

### 4.3 Decision

MIT is preserved. No dependency creates a legal incompatibility with MIT distribution. `THIRD_PARTY_NOTICES.md` was added because the project distributes compiled binaries that include Apache-2.0 components, which require attribution.

---

## 5. Dependency and supply-chain audit

### 5.1 npm vulnerabilities

**Status: remediated.** At audit time `npm audit` reported 22 vulnerabilities (2 critical, 16 high, 3 moderate, 1 low). All were resolved by `npm audit fix` without `--force`; the current count is **0**. Full triage, dependency paths, and exploitability analysis are in [`DEPENDENCY_VULNERABILITY_REPORT.md`](DEPENDENCY_VULNERABILITY_REPORT.md).

The original table is retained below as the historical baseline.

| Package | Severity | Direct |
| --- | --- | --- |
| `vitest` | critical | yes |
| `handlebars` | critical | no (transitive) |
| `vite` | high | yes |
| `react-router-dom` | high | yes |
| `postcss` | high | yes |
| `zod` | high | yes |
| `rollup`, `ws`, `lodash`, `js-yaml`, `minimatch`, `picomatch`, `nanoid`, `brace-expansion`, `browserslist`, `fast-uri`, `source-map-js` | high | no |
| `@vitest/mocker`, `ajv`, `baseline-browser-mapping` | moderate | no |
| `@babel/core` | low | no |

**Not auto-fixed.** Several fixes require major version bumps (`vite`, `vitest`, `react-router-dom`, `zod`), which the task explicitly forbids doing automatically. This is a maintainer decision (F17).

### 5.2 Rust advisories

**Status: remediated to 2 accepted advisories.** `cargo-audit` 0.23.0 was installed and run. It initially reported 22 vulnerabilities. Removing the unused `zip` dependency, updating the lockfile, and disabling unused `sqlx` features reduced this to 2, both documented with reachability arguments in `src-tauri/.cargo/audit.toml`. See [`DEPENDENCY_VULNERABILITY_REPORT.md`](DEPENDENCY_VULNERABILITY_REPORT.md) §3.

### 5.3 Build and release

- No CI workflow exists (`.github/` was absent). No GitHub Actions permissions to review.
- Release is local via `@programinglive/commiter@^1.5.1` (`commiter release`). No artifact publication, no registry credentials, no executable downloads beyond normal package installs.
- `src-tauri/Cargo.lock` and `package-lock.json` are tracked, which is correct for a binary application.

---

## 6. Documentation accuracy findings

These are the most consequential findings for open-source publication, because they misrepresent what the software does.

| ID | Finding | Evidence | Severity |
| --- | --- | --- | --- |
| F2 | README, QUICKSTART, and IMPLEMENTATION_SUMMARY describe backup, restore, copy, and job history as available. The pages exist but are **not imported anywhere**; `App.tsx` renders only `ConnectionsPage`. | `git grep 'pages/backup'` → 0 hits; `App.tsx:116` renders only `ConnectionsPage`; nav buttons at `App.tsx:42-59` have no `onClick` | High |
| F3 | The internal dumper writes an empty file and reports success. | `mysql.rs:182-220`, `postgres.rs:165-203` | High (code) |
| F4 | `drop_existing` / `drop_in_target` / `stop_on_error` are ignored. | `_opts` at `mysql.rs:226`, `postgres.rs:209` | Medium (code) |
| F5 | UI says "Compress (ZIP)"; the code uses deflate and writes `.sql.deflate`. The `zip` crate is unused. | `backup.tsx:212`, `mysql.rs:165,304`; `git grep 'zip::'` → 0 | Medium |
| F6 | `run_query` executes arbitrary SQL from the frontend. | `mysql.rs:73`, `postgres.rs:87` | Medium (code) |
| F7 | `use_ssl` is declared but never read. | `models.rs:13`; only occurrence | Medium (code) |
| F8 | Passwords are interpolated into connection URLs. | `mysql.rs:22,38,189,326`; `postgres.rs:21,37,172,309` | Medium (code) |
| F10 | Version drift: `App.tsx` shows 0.1.4, `package.json` 0.1.6, `tauri.conf.json` 0.1.0, `Cargo.toml` 0.1.0. | four files | Low |
| F12 | `npm run format` calls Prettier, which is not installed. | `package.json` script; `node_modules/prettier` absent | Low |
| F13 | `GITHUB_SETUP_SUMMARY.md` and `COMMITER_SETUP.md` reference the removed `scripts/release.js` and commiter v1.1.6. | both files | Low |
| F11 | `index.html` title is the Tauri template default. | `index.html:7` | Low |

### 6.1 What was changed

- README rewritten with three explicit capability states and no claim that unreachable features are available.
- QUICKSTART corrected: no ZIP claim, no drop-schema claim, no stop-on-error claim, portable paths, correct `cargo` invocation.
- `index.html` title corrected.
- `npm run format` removed, because the tool it calls is not a dependency.
- Historical banners added to `GITHUB_SETUP_SUMMARY.md`, `IMPLEMENTATION_SUMMARY.md`, and `COMMITER_SETUP.md` rather than rewriting history.

### 6.2 What was deliberately not changed

The code defects F3–F8 are documented, not fixed. Fixing them is runtime work owned by the architecture milestones (M3), and this task explicitly excludes database runtime features.

---

## 7. Repository hygiene

`.gitignore` before: logs, `node_modules`, `dist`, `*.local`, editor files. It did **not** exclude database dumps, snapshots, `.env` files, private keys, or `src-tauri/target/`.

Added patterns (see `.gitignore`): dumps and snapshots, `.env*` with an exception for `.env.example`, key material, local connection config, `src-tauri/target/`, coverage, and OS files. Legitimate fixtures, schemas, migrations, and `.env.example` remain trackable.

---

## 8. Verification executed

| Check | Command | Result |
| --- | --- | --- |
| Frontend tests | `npm test` | 3 files, 10 tests passed |
| Pre-commit gate | `node scripts/verify-tests.cjs` | passed |
| Rust tests | `cargo test` (in `src-tauri`) | 0 tests, build succeeded |
| Secret scan (tracked) | `git grep` over 13 patterns | 0 live secrets |
| Secret scan (history) | `git log --all -p` | 0 live secrets |
| npm audit | `npm audit --json` | 22 vulnerabilities (2 critical, 16 high, 3 moderate, 1 low) |
| Cargo license inventory | `cargo metadata` | 646 packages, all permissive |
| Markdown link check | custom Node script over 10 documents | 11 relative links checked, 0 broken |

---

## 9. Unresolved risks

1. **No CI.** Nothing prevents a future commit from introducing a secret or breaking the build. Recommended: a least-privilege workflow running `npm test`, `cargo test`, and a secret scanner on pull requests.
2. **No `cargo-audit` run.** Rust advisory status is unknown.
3. **22 npm vulnerabilities**, including 2 critical. Not fixed because the fixes are major upgrades.
4. **`dlopen2` license field missing** in crate metadata.
5. **No Laravel adapter in this repository.** The task brief references one; it is not present here. If it lives in another repository, that repository needs its own audit. No claim about it is made in the README.
6. **Commercial UI in an open-source repository.** `App.tsx` shows "You are using the free trial", "Purchase", and "Activate". This is a positioning decision for the maintainer, not a defect.
7. **Logo provenance.** `public/logo_dbpinball.png` (208 KB) has no recorded author or license. It is treated as a project asset and not relicensed.
8. **Pre-commit hook depends on an untracked script.** `.husky/pre-commit` runs `node scripts/verify-tests.cjs`, but that file is untracked. If the hook is committed without the script, every commit fails for every contributor. The script must be committed together with the hook, or the hook must be reverted to `npm test`.

---

## 10. Working-tree status

Preserved, not committed:

| Path | State |
| --- | --- |
| `.husky/pre-commit` | modified by the owner (runs `scripts/verify-tests.cjs`) |
| `package.json` | modified by the owner (commiter `^1.5.1`, release scripts) |
| `package-lock.json` | modified by the owner's install |
| `scripts/verify-tests.cjs` | untracked, owner-authored |
| `scripts/release.cjs` | deleted by the owner-requested commiter migration; backup at `%TEMP%\opencode\dbpinball-release.cjs.bak` |
| `docs/ARCHITECTURE-AUDIT.md` | new, from the architecture task |
| `docs/superpowers/specs/…` | new, from the architecture task |

This task adds documentation and hygiene files only. It does not commit, push, tag, or rewrite history.

---

## 11. Maintainer decisions required

| # | Decision | Why it needs a human |
| --- | --- | --- |
| D1 | Confirm the copyright holder string in `LICENSE` | Legal ownership cannot be inferred from a GitHub handle |
| D2 | Decide whether to upgrade `vite`, `vitest`, `react-router-dom`, `zod` to clear the critical/high advisories | Major version bumps |
| D3 | Decide whether the trial/purchase UI stays in the open-source build | Product positioning |
| D4 | Confirm the license and author of `public/logo_dbpinball.png` | Asset provenance |
| D5 | Configure GitHub private vulnerability reporting, or provide a security contact | `SECURITY.md` cannot invent an address |
| D6 | Decide whether the unreachable backup/restore/copy pages should be wired or removed | Product scope |
| D7 | Confirm whether a Laravel adapter exists in another repository and needs its own audit | Out of this repository's scope |

---

## 12. Hardening update (2026-11-12)

### 12.1 Dependency remediation

npm: 22 → 0. Cargo: 22 → 2 accepted. Details in [`DEPENDENCY_VULNERABILITY_REPORT.md`](DEPENDENCY_VULNERABILITY_REPORT.md).

### 12.2 Trial, purchase, and activation UI (P3)

`src/App.tsx:28-38` renders:

- `You are using the free trial`
- a `Purchase` button
- an `Activate` button

**Finding: these are presentational placeholders, not licensing enforcement.**

Evidence:

- `git grep -i -E 'trial|purchase|activat|licen[cs]e|subscription|entitlement|paywall|stripe|paddle'` over `src/` and `src-tauri/src/` returns only the three `App.tsx` lines and two unrelated CSS `stripe` comments.
- Neither button has an `onClick` handler.
- There is no entitlement check, no activation flow, no key validation, no network call, and no gating of any feature.

**No functional licensing code exists, so nothing was removed.** Removing the controls is a product decision, not a security fix.

Recommended open-source model: remove the trial/purchase/activate controls from the open-source build, keep the MIT core unrestricted, and if a commercial edition is planned, implement it as a separate optional module that does not gate core database functionality. Do not add restrictions to the MIT-licensed core.

### 12.3 Unreachable pages (P4)

| Page | Implementation state | Backend | Reachable |
| --- | --- | --- | --- |
| `src/pages/backup.tsx` | Complete UI, calls `invoke("backup")` | `backup` command exists | **No** — not imported |
| `src/pages/restore.tsx` | Complete UI, calls `invoke("restore")` | `restore` command exists | **No** — not imported |
| `src/pages/copy.tsx` | Complete UI, calls `invoke("copy_db")` | `copy_db` command exists | **No** — not imported |
| `src/pages/jobs.tsx` | UI over `localStorage` only; "download" is an `alert` placeholder | No backend | **No** — not imported |

`App.tsx` renders only `ConnectionsPage`. The nav-rail "Backup database…" and "Restore databases…" buttons have no handlers.

**The pages were not deleted.** They are preserved for the CLI-first integration. They are documented as disconnected in the README and must not be presented as released features.

Security note: because the pages are unreachable, the unrestricted `run_query` and the ignored `drop_existing` are not currently exposed through them. They remain defects in the backend commands.

### 12.4 License and asset verification (P5)

| Item | Finding |
| --- | --- |
| `dlopen2` | Resolved to 0.8.2, which declares `license = "MIT"`. The earlier 0.8.0 used a `LICENSE` file containing the MIT License, Copyright (c) 2017 Szymon Wieloch. **MIT confirmed.** |
| `dlopen2_derive` | Resolved to 0.4.3, MIT. **MIT confirmed.** |
| `public/logo_dbpinball.png` | 208 KB PNG with no `tEXt`/`iTXt`/`zTXt` metadata, no author, no license, no copyright chunk. **Ownership unverified.** It is treated as a project asset and is **not** claimed to be MIT-licensed. The maintainer must confirm its origin before redistribution. |

### 12.5 CI and pre-commit

- `.github/workflows/ci.yml` added: least-privilege `contents: read`, `npm ci`, lint, test, build, Rust fmt/clippy/test, `npm audit --audit-level=high`, `rustsec/audit-check`, and gitleaks.
- `.husky/pre-commit` now falls back to `npm test` when `scripts/verify-tests.cjs` is absent, so a fresh clone works whether or not the script is committed.
- Release path verified: `commiter release` with `@programinglive/commiter@1.5.1`; no local `scripts/release.cjs`.
