# Security CLI

The `dbpinball security` commands verify destinations and evaluate the policy
headlessly. Every decision comes from the Rust core.

## `dbpinball security inspect <id>`

Read the destination identity and compare it to any enrollment.

```bash
dbpinball security inspect <id>
dbpinball security inspect <id> --json
```

JSON shape:

```json
{
  "connectionId": "...",
  "declaredTarget": "local",
  "identity": {
    "engine": "postgres",
    "host": "127.0.0.1",
    "port": 5432,
    "identityKey": "postgres://127.0.0.1:5432/16384#12345",
    "server": { "serverUuid": "12345", "database": "app", "readOnly": false },
    "observedAtMs": 1763000000000
  },
  "assessedCategory": "unknown",
  "enrollment": null,
  "enrollmentMatch": "not_enrolled",
  "writeAllowed": false,
  "decisionCode": "not_enrolled"
}
```

## `dbpinball security enroll <id> --category <development|disposable>`

Verify the destination, then store its fingerprint.

```bash
dbpinball security enroll <id> --category development
```

Refuses when the server did not report an identity, or when the category cannot
be a write target.

## `dbpinball security verify <id>`

Re-verify without changing the enrollment.

```bash
dbpinball security verify <id> --json
```

## `dbpinball security policy-check <id> --operation <op>`

Evaluate the policy for one operation.

| Operation | Class |
| --- | --- |
| `inspect` | `ReadOnlyInspection` |
| `query` | `ReadOnlyQuery` |
| `backup` | `BackupExport` |
| `import` | `LocalDataImport` |
| `restore` | `DatabaseRestore` |
| `copy` | `DatabaseCopy` |
| `schema` | `SchemaMutation` |
| `data` | `DataMutation` |

```bash
dbpinball security policy-check <id> --operation restore --json
```

Exit code `0` means allowed. A refusal exits `4` (`policy_refusal`) and prints
the decision code.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Allowed |
| 2 | Usage error |
| 3 | Connection or credential failure |
| 4 | Policy refusal |
| 5 | Operation or environment failure |
| 6 | Safe Sync committed the rows but the identity sequence needs reconciliation |
| 7 | Unknown connection profile |
| 8 | Store conflict |
| 9 | Store busy |

## Credentials

The commands use the same credential source as the rest of the CLI:
`--credential-source keyring` (default) or `env`. No secret is printed.

## Related

- [Security policy](/security/policy)
- [Destination verification](/security/destination-verification)
