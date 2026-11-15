# Security Policy

## Supported versions

DBPinball is pre-1.0. Security fixes are applied to the latest release.

| Version | Supported |
| --- | --- |
| 0.1.7 (latest release) | Yes |
| Older releases | No |

## Reporting a vulnerability

**Do not open a public GitHub issue for a security vulnerability.**

Use GitHub private vulnerability reporting:

<https://github.com/programinglive/dbpinball/security/advisories/new>

Private reporting is enabled on this repository. If the link is unavailable,
open a public issue titled `Security contact request` that contains **no
vulnerability details**, no credentials and no production data, and wait for a
private channel before sharing anything sensitive.

There is no published security email address. Do not invent one, and do not send
reports to addresses you find elsewhere.

### What to include

- A description of the issue and its impact.
- The affected version or commit.
- Reproduction steps using **synthetic** data.
- The affected component: core, CLI, desktop adapter, keyring handling, or the
  Safe Sync pipeline.
- Any suggested fix.

### What not to include

- Real credentials, tokens or private keys.
- Real production dumps or customer records.
- Live exploit payloads against systems you do not own.
- Screenshots containing real connection strings or personal data.

If a proof of concept needs real data, describe the shape of the data instead of
attaching it.

## Disclosure process

- Reports are handled privately until a fix is available.
- You receive an acknowledgement, and updates as the fix progresses.
- Credit is given in the advisory unless you ask to remain anonymous.
- Please do not disclose the issue publicly before a fix ships.

## What qualifies

In scope:

- The Rust core (`crates/dbpinball-core`), including Safe Sync planning,
  masking, plan authorization and the import path.
- The `dbpinball` CLI.
- The desktop adapter and its Tauri commands.
- Credential storage and keyring handling.
- The release artifacts and their provenance.

Out of scope:

- Vulnerabilities in PostgreSQL, MariaDB, or the vendor client tools.
- Vulnerabilities in third-party dependencies with no available fix; report them
  upstream, and we will track them.
- Issues that require an attacker to already have full control of the user's
  machine.

## Database safety

DBPinball connects to real databases, so a defect can destroy data. Treat any
issue that could cause an unintended write as a security issue.

### Safe Sync boundaries

Safe Sync is implemented for PostgreSQL and MariaDB 10.11+ InnoDB. It:

- opens the source session read-only at the database level and refuses a source
  role that can write;
- requires the destination to be enrolled as local, and binds its identity
  fingerprint into the plan, so a connection labelled "local" that actually
  points at a remote server is refused;
- signs the plan with an HMAC over its content, issuance time and expiry, so an
  edited, re-dated or re-keyed plan is refused;
- refuses non-InnoDB tables, triggers, foreign-key cycles, unsupported column
  types, and masking strategies that are not implemented;
- writes inside one transaction and rolls back on any failure.

### Masking is pseudonymization, not anonymization

Masking uses a keyed HMAC. Anyone who holds the masking key can re-identify the
data. The key is generated with a CSPRNG and stored in the OS keyring. Masking
cannot make data safe if the key is retained or shared, if a column is
misclassified, if free text contains values no policy covers, or if the output
is combined with another dataset that re-identifies it.

### Known limitations

These are confirmed behaviours, documented so users can make informed decisions:

1. **The internal dumper fallback is incomplete.** When `mysqldump` or `pg_dump`
   is unavailable, the fallback creates an empty file and reports success.
2. **Passwords are interpolated into connection URLs.** The desktop adapter builds
   `postgresql://user:password@host/db` and `mysql://user:password@host/db` by
   string formatting in the query connection helpers and in the fallback
   dumpers. A password containing `@`, `:`, `/`, `?` or `#` can produce a
   malformed URL, and a database error message may echo the URL. The restore path
   passes the password through `PGPASSWORD`/`MYSQL_PWD` instead, and the Core Safe
   Sync path builds its connections from separate connect options.
3. **The `useSsl` flag is stored but not applied** to connections.
4. **Backup, restore and copy are not routed in the UI**, and the CLI has no
   backup command. Do not rely on them.
5. **Four masking strategies are declared but not implemented**
   (`deterministic_phone`, `date_shift`, `amount_jitter`, `fixed`). A policy that
   selects one is refused.
6. **Non-MariaDB MySQL is refused for Safe Sync.**

Do not point DBPinball at production data for anything other than a read-only
inspection or a reviewed Safe Sync plan.

## Repository security controls

- CI runs `cargo audit`, `npm audit`, a dependency-boundary check and a gitleaks
  secret scan on every pull request and on `main`.
- GitHub secret scanning and push protection are enabled.
- Dependabot alerts and security updates are enabled.
- Release artifacts publish `SHA256SUMS-*.txt` and `provenance.json` recording
  the source commit and toolchain.

## Release integrity

- Every release is built by CI from a tagged commit on `main`.
- `provenance.json` records the exact commit, version, architecture and
  toolchain.
- The Windows binaries are **not code-signed**. Verify the published SHA-256
  before running an installer or archive.
