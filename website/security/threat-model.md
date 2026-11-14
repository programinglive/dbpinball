# Threat model

This page lists what the M3 policy defends against, and what it does not.

## In scope

| Threat | Defence |
| --- | --- |
| Production destination mislabeled `local` | The label is ignored; the server identity must match an enrollment |
| Staging destination mislabeled `development` | Same: the fingerprint decides, not the label |
| Localhost SSH tunnel to production | The tunnel changes the endpoint, not the server identity; the fingerprint will not match |
| Private-network production server | Private IP ranges are not evidence; the fingerprint decides |
| Same source and destination | `same_source_and_destination` |
| Connection string changed after enrollment | `destination_changed` |
| Database renamed or replaced | `destination_changed` |
| Expired authorization | `stale_verification` |
| Writable CTE | Classified as a write |
| Multi-statement SQL | Strictest verdict across statements |
| `SELECT ... INTO` | Classified as a write |
| Stored procedure with side effects | `CALL`/`DO` classified as unknown, treated as a write |
| Restore into an unknown database | `not_enrolled` or `unverifiable_destination` |
| Copy with reversed source and destination | `same_source_and_destination` |
| Policy bypass through the desktop | The query runner calls the core before executing |
| Policy bypass through the CLI | The CLI calls the same core service |
| Credential leakage in output | Errors are redacted; fingerprints contain no secrets |

## Out of scope

| Threat | Why |
| --- | --- |
| A user who deliberately enrolls a production database | Enrollment is an explicit decision; the policy records it, it cannot read intent |
| A compromised database server that lies about its identity | The policy trusts the server's self-report |
| A malicious DBA granting write privileges | Application policy cannot replace database permissions |
| SQL that is read-only at classification time but writes through a function | The classifier is a risk signal, not a sandbox |
| Network interception between verification and execution | The freshness window bounds it; it does not eliminate it |
| A cloned database with a copied fingerprint | The clone has a different identity; enrolling it is a deliberate act |

## The honest summary

M3 makes a write require an independently observed identity that matches an
explicit enrollment, and it fails closed when that cannot be established. It does
not make a database safe by itself. Least-privilege credentials and database
permissions remain the only enforcement the engine cannot be talked out of.

## Related

- [Security policy](/security/policy)
- [Destination verification](/security/destination-verification)
- [Limitations](/security/limitations)
