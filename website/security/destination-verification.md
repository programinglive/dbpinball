# Destination verification

A write is allowed only when the destination is independently identified, matches
a fingerprint the user enrolled, and the evidence is fresh.

## What the server reports

DBPinball asks the database what it is. Nothing is inferred from the profile.

| Fact | PostgreSQL | MySQL |
| --- | --- | --- |
| Engine | `version()` | `VERSION()` |
| Server identity | `pg_control_system().system_identifier` | `@@server_uuid` |
| Database | `current_database()` | `DATABASE()` |
| Database id | `pg_database.oid` | schema name |
| Current user | `current_user` | `CURRENT_USER()` |
| Read-only session | `transaction_read_only` | `@@read_only` |

`pg_control_system()` requires elevated privileges. On PostgreSQL 18 a plain
login role can run it, so the identity is practical for ordinary developer
credentials. On older releases or hardened clusters it may be denied; when it is
unavailable the server identity is missing, and the policy treats the destination
as **unverifiable** rather than inventing one.

## The fingerprint

```
<engine>://<host>:<port>/<database-id>#<server-identity>
```

The fingerprint contains no password, token, or full connection string. It is
stored in `<config>/dbpinball/security/enrollments.json`.

## Categories

| Category | Allows writes |
| --- | --- |
| `VerifiedDisposable` | yes |
| `VerifiedDevelopment` | yes |
| `Unknown` | **no** |
| `ProductionLike` | **no** |
| `Denied` | **no** |

`Unknown` is never upgraded to development. If the destination cannot be
classified, dangerous writes are refused.

## Enrollment

Enrollment is deliberate and scoped to one database identity.

```bash
dbpinball security enroll --connection <id> --category development
```

The command verifies the destination first, then stores the fingerprint. It
refuses when the server did not report an identity, and it refuses a category
that cannot be a write target.

## Freshness

Verification evidence expires. The default window is ten minutes. A stale
observation returns `require_verification`, not `allow`.

Re-verify immediately before a dangerous operation:

```bash
dbpinball security verify --connection <id>
```

## What changes invalidate an enrollment

| Change | Result |
| --- | --- |
| Server replaced (new `system_identifier` / `server_uuid`) | `destination_changed` |
| Database renamed or recreated | `destination_changed` |
| Connection string points at another host | `destination_changed` |
| Evidence older than the window | `stale_verification` |

## Limitations

- **Cloned databases.** A clone has the same schema but a different server
  identity. A copied fingerprint does not authorize the clone; the user must
  enroll it deliberately.
- **Privilege escalation.** Application policy cannot replace database
  permissions. A least-privilege role is still the only thing that stops a write
  at the engine.
- **TOCTOU.** Verification and execution are separate round-trips. The freshness
  window bounds the gap; it does not eliminate it.
- **Unix sockets.** When `inet_server_addr()` is empty, the configured host is
  used, which is weaker evidence.

## Related

- [Security policy](/security/policy)
- [Threat model](/security/threat-model)
