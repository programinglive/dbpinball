# Credentials and keyring

DBPinball never stores a password in the connection store. Passwords live in the
operating system keyring.

## Where credentials live

| Platform | Backend |
| --- | --- |
| Windows | Credential Manager |
| macOS | Keychain |
| Linux | libsecret (GNOME Keyring, KWallet, or another Secret Service provider) |

The keyring entry uses:

- **service**: `dbpinball`
- **account**: the connection profile id

The desktop and the CLI use the same service and account, so a credential stored
by one is found by the other.

## Credential sources in the CLI

| Source | Flag | Can store | Can delete |
| --- | --- | --- | --- |
| Keyring (default) | `--credential-source keyring` | Yes | Yes |
| Environment | `--credential-source env` | No | No |

The environment source reads `DBPINBALL_PASSWORD`. It is intended for CI and
automation. It is **not** a persistent replacement for the desktop keyring: it
is per-invocation and cannot store or delete a credential.

## What happens when a credential is missing

- The profile still exists and is listed.
- `connections test` fails with exit code 3 (`connection`).
- The desktop shows the profile and asks for the password.

## What happens when the keyring is unavailable

- The command fails with exit code 3; there is no plaintext fallback.
- The profile metadata is not removed.
- The desktop reports a safe error and keeps the profile.

## Migration and credentials

The desktop migration from the old browser `localStorage` imports **metadata
only**. It never reads, writes or regenerates a password. Existing keyring
entries keep working because the service and account are unchanged.

## Related

- [No plaintext fallback](/security/no-plaintext-fallback)
- [OS keyring integration](/security/keyring)
- [Credential lifecycle](/architecture/credential-lifecycle)
