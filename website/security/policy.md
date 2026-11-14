# Security policy

DBPinball decides whether a database operation may run. The decision is made in
the Rust core, not in the CLI, the desktop, or a confirmation dialog.

## The rule

> A user-declared connection classification is never accepted as proof that a
> destination is safe to write.

A profile named `local`, a hostname of `127.0.0.1`, a database called `dev`, or a
click on **Confirm** are all inputs. None of them authorizes a write.

## Operation classes

Every operation is classified before it runs.

| Class | Writes? | Notes |
| --- | --- | --- |
| `ReadOnlyInspection` | no | Listing databases, tables, connection tests |
| `ReadOnlyQuery` | no | A statement the classifier proved read-only |
| `BackupExport` | no | Dumping a database to a file |
| `LocalDataImport` | **yes** | Loading a file into a database |
| `DatabaseRestore` | **yes** | Restoring a dump |
| `DatabaseCopy` | **yes** | Copying one database into another |
| `SchemaMutation` | **yes** | DDL |
| `DataMutation` | **yes** | DML |
| `SafeSyncPrepare` | reads a source | M4, not implemented |
| `SafeSyncExecute` | **yes** | M4, not implemented |

`SafeSyncPrepare` and `SafeSyncExecute` are refused with `unsupported` until M4
ships.

## SQL classification

A statement is not read-only because it starts with `SELECT` or `WITH`. The
classifier:

- strips comments and string literals first, so a keyword or `;` inside a literal
  cannot change the verdict;
- splits on `;` and takes the strictest verdict across statements;
- treats a `WITH` statement as a write when it contains `INSERT`, `UPDATE`,
  `DELETE`, or `MERGE`;
- treats `SELECT ... INTO` as a write;
- treats `EXPLAIN ANALYZE` as unknown, because it executes the statement;
- treats `CALL`, `DO`, `EXECUTE`, and `PREPARE` as unknown, because they can run
  arbitrary code;
- returns `Unknown` for anything it does not recognize.

`Unknown` is treated as a write. The classifier is a risk signal, not a security
boundary: the database's own permissions are still the last line of defence.

## The decision

```rust
pub enum PolicyDecision {
    Allow { code: PolicyCode, evidence: String },
    Deny { code: PolicyCode, reason: String },
    RequireVerification { code: PolicyCode, reason: String },
}
```

The evaluation order:

1. Safe Sync → `unsupported`.
2. Read-only → `allow`.
3. No destination identity → `deny` (`missing_destination`).
4. Server did not identify itself → `deny` (`unverifiable_destination`).
5. Source and destination are the same database → `deny`
   (`same_source_and_destination`).
6. No enrollment → `require_verification` (`not_enrolled`).
7. Fingerprint mismatch → `deny` (`destination_changed`).
8. Evidence older than the freshness policy → `require_verification`
   (`stale_verification`).
9. Category does not allow writes → `deny` (`destination_not_allowed`).
10. Connection declared production → `deny` (`protected_source`).
11. Otherwise → `allow`.

Every refusal is a stable code, so a script can branch on it.

## Where it is enforced

| Layer | Enforcement |
| --- | --- |
| Core | `dbpinball_core::security::evaluate` |
| CLI | `dbpinball security policy-check` |
| Desktop query | Classified before execution; writes need a verified destination |
| Desktop restore | `authorize_write` before the client runs |
| Desktop copy | `authorize_copy` verifies both sides before the dump starts |
| Backup | Read-only export, unchanged |

The desktop cannot bypass the policy: the query runner classifies the SQL and
calls the core before executing anything that can write, and restore and copy are
gated at the same execution boundary. A command is not safe because its UI page
is unrouted; the gate is on the command itself.

## Read-only execution

A statement the classifier calls read-only is executed inside a database-level
read-only transaction:

- **PostgreSQL**: `SET TRANSACTION READ ONLY`. The engine refuses DML and DDL
  inside it.
- **MySQL**: `SET TRANSACTION READ ONLY` on the same connection that runs the
  statement. The engine refuses DML on transactional tables (error 1792).
  **DDL is not covered**: MySQL performs an implicit commit before DDL, which
  ends the read-only transaction. The classifier never sends DDL down this path,
  but the engine guarantee is narrower than PostgreSQL's.

The classifier is a risk signal; the engine is the boundary. Neither is a
sandbox for functions with external side effects.


## Related

- [Destination verification](/security/destination-verification)
- [Threat model](/security/threat-model)
- [Security CLI](/reference/security-cli)
