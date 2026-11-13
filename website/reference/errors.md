# Error behaviour

## Error codes

| Code | CLI exit | Meaning |
| --- | --- | --- |
| `usage` | 2 | Invalid arguments or an invalid profile |
| `connection` | 3 | Connection or credential failure |
| `policy_refusal` | 4 | A safety rule refused the action |
| `operation` | 5 | Filesystem or environment failure |
| `unsupported` | 5 | Unsupported store version |
| `not_found` | 7 | Unknown connection profile |
| `conflict` | 8 | The store changed since it was read |
| `busy` | 9 | The store lock was not acquired in time |

## Common messages

| Message | Cause | Fix |
| --- | --- | --- |
| `a connection with this id already exists` | Duplicate id on create | Use a new id, or update the existing profile |
| `the connection store changed since it was read` | Stale revision | Reload and retry |
| `the connection store is busy` | Another writer holds the lock | Retry |
| `connection store is not valid JSON` | Corrupt store file | Move the file aside and start fresh |
| `connection store version N is not supported` | Newer store written by a newer build | Upgrade the build |
| `operating system keyring is unavailable` | No keyring backend | Unlock the keyring, or use the env source |
| `credential could not be read for this connection` | Missing keyring entry | Set the password again |

## Redaction

Every message passes through `redact`, which removes `user:password@` from
URL-shaped text. The desktop maps codes to fixed messages, so a raw core message
never reaches the UI.

## Related

- [Troubleshooting](/guide/troubleshooting)
- [Redaction guarantees](/security/redaction)
