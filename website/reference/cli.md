# CLI commands

Every command supports the global flags `--json`, `--config-dir <PATH>` and
`--credential-source <keyring|env>`.

## `dbpinball doctor`

Check the local environment.

```bash
dbpinball doctor
dbpinball doctor --json
```

## `dbpinball connections list`

List stored profiles. Never prints passwords.

```bash
dbpinball connections list
dbpinball connections list --json
```

## `dbpinball connections add`

Add a profile. The password is read from stdin.

| Flag | Required | Meaning |
| --- | --- | --- |
| `--name` | yes | Profile name |
| `--engine` | yes | `mysql` or `postgres` |
| `--host` | yes | Database host |
| `--port` | yes | Database port |
| `--database` | yes | Database name |
| `--username` | yes | Database user |
| `--target` | no | `production`, `staging`, `local`, `unknown` (default `unknown`) |
| `--use-ssl` | no | Store the SSL preference |

```bash
dbpinball connections add --name local --engine postgres \
  --host 127.0.0.1 --port 5432 --database app --username postgres --target local
```

## `dbpinball connections set-password <id>`

Replace the stored password. The password is read from stdin.

## `dbpinball connections test <id>`

Test a profile against its database.

## `dbpinball connections remove <id> [--yes]`

Remove a profile and its stored credential. `--yes` is required when stdin is not
a terminal.

## `dbpinball connections import <file>`

Import profiles from a JSON file. The file may be an array of profiles or a full
store object.

## `dbpinball schema inspect`

| Flag | Required | Meaning |
| --- | --- | --- |
| `--connection` | yes | Profile id |
| `--database` | no | List tables in this database instead of listing databases |

```bash
dbpinball schema inspect --connection <id>
dbpinball schema inspect --connection <id> --database app
```

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 2 | Usage error |
| 3 | Connection or credential failure |
| 4 | Policy refusal |
| 5 | Operation or environment failure |
| 7 | Unknown connection profile |
| 8 | Store conflict |
| 9 | Store busy |
