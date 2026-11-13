# Release Readiness Checklist — DBPinball

Date: 2026-11-12
Version under review: `0.1.6` (working tree, uncommitted)
Purpose: evidence that the repository can be published and built by an outside contributor.

Every item below was executed in this environment. Results are recorded as observed, not assumed.

---

## 1. Build and test

| # | Check | Command | Result |
| --- | --- | --- | --- |
| 1.1 | Fresh install from the working tree | `npm ci` in a clean copy | passed |
| 1.2 | Frontend tests | `npm test` | 3 files, 10 tests passed |
| 1.3 | Type check | `npm run lint` | clean |
| 1.4 | Frontend production build | `npm run build` | succeeded, `dist/` emitted |
| 1.5 | Rust build and tests | `cargo test` in `src-tauri` | 0 tests, build succeeded |
| 1.6 | Rust formatting | `cargo fmt --all -- --check` | clean |
| 1.7 | Rust lint | `cargo clippy --all-targets -- -D warnings` | clean |
| 1.8 | Pre-commit hook, script present | `sh .husky/pre-commit` | passed |
| 1.9 | Pre-commit hook, script absent | `sh .husky/pre-commit` in a copy without the script | fell back to `npm test`, passed |

**Known gap:** the committed tree at `82a6a52` does **not** build from a fresh clone, because its `package-lock.json` is missing `@testing-library/dom`. The working tree fixes this. The fix must be committed before publication.

## 2. Security

| # | Check | Command | Result |
| --- | --- | --- | --- |
| 2.1 | npm audit, all | `npm audit` | 0 vulnerabilities |
| 2.2 | npm audit, production only | `npm audit --omit=dev` | 0 vulnerabilities |
| 2.3 | Rust advisory audit | `cargo audit` | exit 0; 2 accepted advisories with written reasons |
| 2.4 | Secret scan, tracked files | `git grep` over 13 patterns | 0 live secrets |
| 2.5 | Secret scan, history | `git log --all -p` | 0 live secrets |
| 2.6 | Sensitive artifacts | filesystem scan for dumps, `.env`, keys | none tracked or untracked |
| 2.7 | Internal infrastructure | `git grep` for internal hosts and private IPs | none |

Accepted advisories are documented in `src-tauri/.cargo/audit.toml` with reachability arguments. See `docs/security/DEPENDENCY_VULNERABILITY_REPORT.md`.

## 3. Licensing

| # | Check | Result |
| --- | --- | --- |
| 3.1 | Root `LICENSE` | present, MIT |
| 3.2 | `package.json` `license` | `MIT` |
| 3.3 | `src-tauri/Cargo.toml` `license` | `MIT` |
| 3.4 | Dependency licenses | all permissive; no GPL/AGPL/SSPL |
| 3.5 | `dlopen2` / `dlopen2_derive` | MIT confirmed |
| 3.6 | `THIRD_PARTY_NOTICES.md` | present, direct dependencies listed |
| 3.7 | `public/logo_dbpinball.png` | **ownership unverified** — not claimed as MIT |

## 4. CI

| # | Check | Result |
| --- | --- | --- |
| 4.1 | Workflow file | `.github/workflows/ci.yml` present |
| 4.2 | Permissions | `contents: read` at workflow level |
| 4.3 | Install | `npm ci` |
| 4.4 | Frontend | lint, test, build |
| 4.5 | Rust | fmt check, clippy, test |
| 4.6 | Dependency scan | `npm audit --audit-level=high`, `rustsec/audit-check` |
| 4.7 | Secret scan | gitleaks |
| 4.8 | Caching | `setup-node` npm cache, `Swatinem/rust-cache` |
| 4.9 | Credentials required | none beyond the automatic `GITHUB_TOKEN` |
| 4.10 | Databases required | none |

**Not validated:** the workflow has not run on GitHub. YAML syntax was reviewed manually; no local Actions runner is available. The first push will be the real test.

## 5. Repository hygiene

| # | Check | Result |
| --- | --- | --- |
| 5.1 | `.gitignore` covers dumps, snapshots, `.env`, keys, `target/` | yes |
| 5.2 | No tracked file newly ignored | verified with `git check-ignore` over `git ls-files` |
| 5.3 | Legitimate fixtures and schemas remain trackable | `*.sql` intentionally not ignored |
| 5.4 | Pre-commit works in a fresh clone | yes, via fallback |
| 5.5 | Release path | `commiter release`, `@programinglive/commiter@1.5.1`, no local release script |

## 6. Documentation accuracy

| # | Check | Result |
| --- | --- | --- |
| 6.1 | README distinguishes Available / In development / Planned | yes |
| 6.2 | CLI not presented as available | yes, explicitly marked not available |
| 6.3 | Safe Sync not presented as available | yes |
| 6.4 | Raw copy not described as sanitized | yes |
| 6.5 | Unreachable pages documented as disconnected | yes |
| 6.6 | Relative Markdown links resolve | 11 checked, 0 broken |
| 6.7 | Stale commands removed (`npm run format`, root `cargo test`) | yes |

## 7. Remaining risks

| # | Risk | Severity | Mitigation |
| --- | --- | --- | --- |
| 7.1 | Committed lockfile lacks `@testing-library/dom`; fresh clone fails until the working tree is committed | High | Commit the updated `package-lock.json` |
| 7.2 | `rsa` Marvin timing advisory (RUSTSEC-2023-0071) | Low | Documented acceptance; revisit with sqlx 0.9 |
| 7.3 | `libsqlite3-sys` in lockfile but not compiled | Low | Documented; re-check after any sqlx feature change |
| 7.4 | CI has never executed on GitHub | Medium | First push validates it |
| 7.5 | Logo ownership unverified | Medium | Maintainer confirmation before redistribution |
| 7.6 | Trial/purchase UI in an open-source build | Low | Product decision |
| 7.7 | Unreachable backup/restore/copy pages | Low | Documented; preserved for CLI-first integration |
| 7.8 | No Rust unit tests | Medium | Add tests with the M1 core extraction |

## 8. Sign-off

Publication is blocked on:

1. Committing the working tree, including the updated `package-lock.json`.
2. Confirming the logo's ownership and license.
3. Confirming the copyright holder string in `LICENSE`.
4. A first successful CI run on GitHub.

Everything else in this checklist is green.
