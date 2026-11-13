# CLI

The `dbpinball` binary is a thin adapter over the core. It parses arguments,
calls the core, and prints either human-readable text or one JSON object.

## Structure

| File | Role |
| --- | --- |
| `src/cli.rs` | Clap definitions for every command and flag |
| `src/commands/connections.rs` | `connections` subcommands |
| `src/commands/doctor.rs` | Environment check |
| `src/commands/schema.rs` | `schema inspect` |
| `src/config.rs` | Resolves the shared configuration directory |
| `src/credentials.rs` | Selects the keyring or environment provider |
| `src/exit.rs` | Maps `ErrorCode` to a process exit code |
| `src/output.rs` | JSON envelope and human output |

## Design rules

- **No password flag.** The password is read from stdin, so it never appears in
  shell history or process arguments.
- **One JSON object.** `--json` emits exactly one object on stdout.
- **Stable exit codes.** The mapping is documented and tested.
- **No Tauri dependency.** The CLI depends on the core, never on the desktop.

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

## Related

- [CLI basics](/guide/cli)
- [CLI reference](/reference/cli)
