# Security Policy

## Supported versions

DBPinball is pre-1.0 and ships from `main`. Security fixes are applied to the latest release only.

| Version | Supported |
| --- | --- |
| Latest release on `main` | Yes |
| Older releases | No |

## Reporting a vulnerability

**Do not open a public GitHub issue for a security vulnerability.**

### Preferred channel

Use GitHub private vulnerability reporting:

<https://github.com/programinglive/dbpinball/security/advisories/new>

This channel only works if the maintainer has enabled private vulnerability reporting in the repository settings (**Settings → Code security → Private vulnerability reporting**). If the link returns 404, the feature is not enabled.

### If private reporting is unavailable

Open a public issue titled `Security contact request` that contains **no vulnerability details**, no credentials, and no production data. Ask the maintainer to enable private vulnerability reporting or to provide a private channel. Wait for a response before sharing anything sensitive.

There is no published security email address. Do not invent one, and do not send reports to addresses you find elsewhere.

### What to include

- A description of the issue and its impact.
- The affected version or commit.
- Reproduction steps using **synthetic** data.
- The affected component (frontend, Tauri command, runner, keyring handling).
- Any suggested fix, if you have one.

### What not to include

- Real credentials, tokens, or private keys.
- Real production database dumps or customer records.
- Live exploit payloads against systems you do not own.
- Screenshots containing real connection strings or personal data.

If a proof of concept requires real data, describe the shape of the data instead of attaching it.

## Sensitive information handling

- Reports are handled privately until a fix is available.
- Credit is given in the advisory unless you ask to remain anonymous.
- Reporters are asked not to disclose the issue publicly before a fix ships.

## Security update practices

- Fixes are released as patch versions on `main`.
- Advisories are published through GitHub Security Advisories when the issue affects released code.
- There is currently **no CI pipeline and no automated dependency scanning**. This is a known gap, not a guarantee of safety.

## Database connection security

- Passwords are stored in the operating system keyring (macOS Keychain, Windows Credential Manager, Linux libsecret) under the service name `dbpinball`.
- Passwords are never written to the repository, to logs, or to the connection metadata file.
- Connection metadata (host, port, database, username) is stored locally in the application's webview storage.

### Known limitations

These are confirmed defects in the current code, documented so users can make informed decisions:

1. **Arbitrary SQL execution.** The workspace query runner sends SQL to the database without a read-only guard. A `DROP`, `UPDATE`, or `DELETE` typed into the workspace will execute.
2. **Password interpolation.** Connection URLs are built by string formatting, so a password containing `@`, `:`, `/`, or `#` can produce a malformed URL, and database error messages may echo the URL.
3. **TLS toggle not applied.** The `useSsl` flag is stored but not used when connecting.
4. **Internal dumper is incomplete.** When `mysqldump` or `pg_dump` is unavailable, the fallback creates an empty file and reports success.

Do not point DBPinball at production data until these are fixed.

## Safe Sync security boundaries

Safe Sync is **not implemented**. The following describes the intended boundary so that no one mistakes the current raw copy behaviour for it.

When implemented, Safe Sync will:

- Write only to a **verified development target**, never to production, staging, or an unclassified target.
- Enforce read-only access on the source at the database level, not by inspecting SQL keywords.
- Verify destination identity against a recorded fingerprint, so a connection labelled "local" that actually points at a remote server is refused.
- Refuse undeclared SSH tunnels, because a tunnel to production binds to `127.0.0.1` and is indistinguishable from a local server by address alone.
- Fail closed when a column cannot be classified as sensitive or non-sensitive.
- Validate referential integrity and scan for residual sensitive values before a snapshot is made available.

**Raw backup, restore, and copy are not Safe Sync.** They are byte-faithful and unsanitized. A raw copy of production data is not safe to share, commit, or attach to an issue.

## Masking and pseudonymization limitations

The planned masking design uses HMAC-SHA256 as a deterministic pseudonymization primitive, combined with synthetic value generation for personal data.

**Pseudonymization is not anonymization.** Anyone who holds the masking key can re-identify the data. The design therefore requires the key to be generated with a CSPRNG, stored in the OS keyring, and destroyed after validation. A retained key is a re-identification risk and is reported in the run summary.

Masking cannot make data safe if:

- the key is retained or shared,
- a column is misclassified as non-sensitive,
- free-text or JSON fields contain sensitive values that no policy covers,
- the snapshot is combined with another dataset that re-identifies it.

## Scope

In scope:

- The desktop application and its Tauri commands.
- Credential storage and keyring handling.
- The Rust database runners.
- The planned CLI and Safe Sync pipeline.

Out of scope:

- Vulnerabilities in MySQL, PostgreSQL, or the vendor client tools.
- Vulnerabilities in third-party dependencies that have no available fix (report them upstream; we will track them).
- Issues that require an attacker to already have full control of the user's machine.
