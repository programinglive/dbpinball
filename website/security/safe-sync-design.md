# Safe Sync design (planned)

Safe Sync is **planned for M4 and is not available**. There is no command, page,
or binary surface for it yet. This page records the approved design so that it can
be reviewed before any runtime code exists.

If you are looking for what exists today, see the
[security policy](/security/policy) and the [roadmap](/roadmap).

## What it is meant to do

A developer needs a small, representative slice of production PostgreSQL data to
reproduce a bug locally, without copying unmasked personal data into the
development environment. M4 is a one-shot workflow that:

- reads a bounded, explicitly selected subset from a production source;
- follows foreign keys so the subset is complete;
- masks sensitive values before they leave the extraction step;
- writes once into a verified development destination.

It is a debugging aid. It is not a cloning tool, not replication, and not a
background sync.

## What it will not do

- It will not clone a production database wholesale.
- It will not write to the production source.
- It will not import unmasked data by default, and it will not fall back to
  unmasked data.
- It will not run user-supplied SQL, expressions, or transformation code.
- It will not change production schema.
- It will not sync in the background or on a schedule.
- It will not skip destination enrollment.
- It will not support MySQL in M4. PostgreSQL only.

## The two safety gates

### 1. The source is read-only at the database, not just in the client

Safe Sync records four separate facts about the source: the verified identity, the
declared role, the role's effective privileges, and the enforcement actually in
place. Only the last is a guarantee.

The source session runs with `default_transaction_read_only = on` and every
extraction transaction is read-only. The source role must be `SELECT`-only on the
selected tables, and a role that is a superuser or that can bypass row-level
security is refused. A client-side flag alone is never treated as proof.

### 2. Masking is mandatory and fails closed

Every selected column needs an explicit masking decision. A column with no rule is
refused, not copied. Columns whose content cannot be inspected safely — JSON,
binary, XML, arrays — are refused unless an explicit redaction rule exists. Free
text, tokens, password hashes and government identifiers are never copied
unchanged.

Output is **de-identified test data, not anonymous data**. Deterministic masking
keeps relationships consistent, but a low-entropy value can still be re-linked by
frequency analysis. The design refuses or redacts those columns rather than
pretending the result is anonymous.

## Verification and execution share a session

The most important change from the current security policy is that verification
and execution are no longer separate round-trips. Safe Sync opens one owned
connection per side, reads the server identity on that connection, and runs the
operation on the same connection. A cached verification cannot authorize a write,
and reconnecting re-establishes authorization.

This removes avoidable connection-substitution risk. It does not remove the
possibility of a malicious server that misreports its identity, and it does not
remove network trust before the first query. Those limits are documented rather
than hidden.

## A plan you can review before anything is written

`plan` produces a JSON document with no row values: the source and destination
identities, the selected tables, the foreign-key graph, the row estimates, the
masking coverage, the constraint strategy, the limits that were checked, and a
plan fingerprint. `execute` requires that fingerprint as an acknowledgement.

The acknowledgement is not authorization. The security policy still decides, and
the destination identity is still checked on the execution session.

## What happens when something is wrong

- If a limit would be exceeded, the plan is marked incomplete and execution is
  refused. Rows are never silently truncated.
- If a masked value would collide on a unique key, the run is refused.
- If a foreign-key cycle cannot be satisfied with deferrable constraints, the run
  is refused rather than disabling constraints.
- If the destination already contains rows in the selected tables, the run is
  refused unless a replace mode is explicitly requested.
- If the plan is replayed after a successful run, it is a no-op, not a duplicate.
- If the import is interrupted, the destination transaction rolls back.

## How it will be available

Safe Sync will be built in the Rust core first and exposed through the CLI, so it
can be tested without the desktop. The desktop will be a thin adapter over the same
core services, with no UI-only security decision.

## Related

- [Security policy](/security/policy)
- [Destination verification](/security/destination-verification)
- [Limitations](/security/limitations)
- [Roadmap](/roadmap)
