# Configuration reference

## Global CLI flags

| Flag | Type | Default | Meaning |
| --- | --- | --- | --- |
| `--json` | boolean | false | Emit exactly one JSON object on stdout |
| `--config-dir` | path | OS config dir | Override the configuration directory |
| `--credential-source` | `keyring` \| `env` | `keyring` | Where passwords are read from |

## Environment variables

| Variable | Read by | Meaning |
| --- | --- | --- |
| `DBPINBALL_PASSWORD` | CLI with `--credential-source env` | The password for the current invocation |

## Files

| File | Location | Purpose |
| --- | --- | --- |
| `connections.json` | `<config>/dbpinball/` | The versioned connection store |
| `connections.lock` | `<config>/dbpinball/` | Cross-process lock, present only during a write |
| `connections.lock.recover` | `<config>/dbpinball/` | Recovery mutex, present only during stale recovery |

## Store constants

| Constant | Value |
| --- | --- |
| `STORE_VERSION` | 1 |
| Keyring service | `dbpinball` |
| Keyring account | connection id |
| Lock wait | 2 s |
| Lock poll | 20 ms |

## Related

- [Profile schema](/reference/profile-schema)
- [Configuration guide](/guide/configuration)
