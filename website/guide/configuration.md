# Configuration

DBPinball has very little configuration. Most of it is the location of the
connection store and the credential source.

## Configuration directory

| Platform | Default |
| --- | --- |
| Windows | `%APPDATA%\dbpinball` |
| macOS | `~/Library/Application Support/dbpinball` |
| Linux | `~/.config/dbpinball` |

The directory is resolved by `dirs::config_dir()/dbpinball` in the Rust core.
Both the desktop and the CLI call the same resolver, so they cannot drift into
two different stores.

The CLI accepts `--config-dir <PATH>` to override it. The desktop always uses the
default.

## Files

| File | Purpose |
| --- | --- |
| `connections.json` | The versioned connection store |
| `connections.lock` | The cross-process lock, present only while a write is in progress |
| `connections.lock.recover` | The recovery mutex, present only while a stale lock is being recovered |

## Environment variables

| Variable | Used by | Meaning |
| --- | --- | --- |
| `DBPINBALL_PASSWORD` | CLI with `--credential-source env` | The password for the current invocation |

`DBPINBALL_PASSWORD` is not a persistent credential store. It cannot store or
delete a password.

## Store schema

See the [profile schema reference](/reference/profile-schema) for the exact
shape of `connections.json`.

## Related

- [Connection store](/architecture/connection-store)
- [Configuration reference](/reference/configuration)
