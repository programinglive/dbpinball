# CLI basics

The `dbpinball` CLI is a standalone binary over the same Rust core as the
desktop. It is designed for scripted and headless inspection.

## Global flags

| Flag | Meaning |
| --- | --- |
| `--json` | Emit exactly one JSON object on stdout |
| `--config-dir <PATH>` | Override the configuration directory |
| `--credential-source <keyring\|env>` | Where passwords are read from (default `keyring`) |

Global flags may appear before or after the subcommand.

## Commands

```text
dbpinball doctor
dbpinball connections list
dbpinball connections add <flags>
dbpinball connections set-password <id>
dbpinball connections test <id>
dbpinball connections remove <id> [--yes]
dbpinball connections import <file>
dbpinball schema inspect --connection <id> [--database <db>]
```

## JSON output

Every command supports `--json`. The output is a single object, so a script can
parse it without handling human-readable text.

```bash
dbpinball connections list --json | jq '.connections[].name'
```

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 2 | Usage error |
| 3 | Connection or credential failure |
| 4 | Policy refusal |
| 5 | Operation or environment failure |
| 6 | Safe Sync committed the rows but the identity sequence needs reconciliation |
| 7 | Unknown connection profile |
| 8 | Store conflict: another writer changed the store since it was read |
| 9 | Store busy: the cross-process lock was not acquired within the bounded wait |

## A scripted workflow

```bash
set -euo pipefail

# Fail early if the environment is not ready.
dbpinball doctor --json > /dev/null

# Create a profile non-interactively.
printf '%s\n' "$DB_PASSWORD" | dbpinball connections add \
  --name ci --engine postgres --host 127.0.0.1 --port 5432 \
  --database app --username postgres --target local

# Inspect and clean up.
dbpinball schema inspect --connection "$(dbpinball connections list --json | jq -r '.connections[0].id')"
dbpinball connections remove "$(dbpinball connections list --json | jq -r '.connections[0].id')" --yes
```

## What the CLI does not do

- It does not run backups, restores or copies.
- It does not implement Safe Sync.
- It does not print passwords.

See the [CLI reference](/reference/cli) for the exact flags of every command.
