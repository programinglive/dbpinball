# Safe Sync design

Safe Sync is **implemented** for PostgreSQL and MariaDB 10.11+ InnoDB. It is
available as dbpinball safe-sync and as the desktop Safe Sync page. This page
records the design and the boundaries the implementation enforces.

For the supported types and the refusals, see the
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

`plan` produces a JSON document with no row values and no filter values: the
source and destination identities, the selected tables, the foreign-key graph, the
row estimates, the masking coverage, the constraint strategy, the limits that were
checked, and a plan fingerprint. Filter values stay in the spec you control; the
plan carries only a keyed digest of them.

`execute` takes both the plan and the spec and requires the plan fingerprint as an
acknowledgement. The acknowledgement is not authorization. The security policy
still decides, and the destination identity is still checked on the execution
session.

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

## What is implemented so far

Safe Sync is still not generally available, but part of it exists on `main`:

- **Slice 1:** one PostgreSQL table, a bounded root filter, mandatory masking,
  bound source and destination sessions, a value-free fingerprinted plan, a
  single transactional destination import, and idempotency verified by a digest
  of the destination rows.
- **S2-a:** foreign-key discovery from the PostgreSQL catalogs, composite and
  self-referencing keys, bounded parent traversal up to a declared depth, stable
  deduplication of parents, canonical key masking so a masked parent key and its
  child foreign key still join, cycle detection with deferrable-only deferral,
  and parent-first import order.

Within S2-a:

- The key-domain material binds the referenced table, the ordered referenced
  columns and the component position, so two different key spaces cannot
  correlate.
- A NULL foreign-key component stays NULL; a partially NULL tuple is hashed with
  a NULL marker so a NOT NULL component is never nulled and cannot match a real
  key. MATCH FULL partial-NULL tuples are refused.
- The plan carries a structural foreign-key fingerprint (name, both sides,
  ordered columns, match type, deferrability) and a per-table schema hash that
  includes relevant CHECK constraints. A changed relationship, CHECK, column or
  nullability invalidates the plan before any destination write.
- Overlapping plans that share destination tables are serialized by the sorted
  table locks; the second refuses with a deterministic conflict instead of
  duplicating or overwriting rows.

Within S2-b:

- Generated columns are omitted from the insert and recomputed by the
  destination. The generated expression is part of the schema hash, so a
  destination with a different expression is refused before any row is written.
- A restricted identity-preserve mode keeps single-column integer identity keys
  with `OVERRIDING SYSTEM VALUE`; regenerating and remapping identities is not
  supported. An integer column may be kept only when the catalogue proves it is
  an identity column or a foreign key that carries a preserved identity value;
  an ordinary integer is refused.
- SQL NULL is preserved as a typed NULL for every strategy. It is never turned
  into an empty string, so a nullable integer is never cast from `''`.
- After the row transaction commits, identity sequences are reconciled in a
  separate, idempotent step. The supported mode is deliberately restricted and
  its safety is enforced, not assumed: an empty table or a `CACHE 1` sequence
  that already covers the imported maximum needs no movement (`not_required`); a
  sequence behind the data is raised to the imported maximum (`completed`); a sequence
  whose state cannot be read, whose lock cannot be taken (missing privilege,
  lock timeout) or whose cache preallocates values is not touched
  (`reconciliation_required`). The read and the `setval` run inside one
  transaction that first changes the sequence with an `ALTER SEQUENCE` whose
  `CACHE` clause repeats the current value. PostgreSQL documents that
  `ALTER SEQUENCE` blocks concurrent
  `nextval`, `currval`, `lastval` and `setval`, and the backend takes a
  `ShareRowExclusiveLock` on the sequence, so the observed `last_value` cannot
  move between the read and the write. The raise is
  `setval(seq, GREATEST(max, last_value), true)`, so it never lowers the value it
  observed.
- Only `CACHE 1` identity sequences are reconciled automatically. A larger cache
  plans ahead by handing out a block of values that this session can neither see
  nor revoke, and the PostgreSQL documentation says other backends keep using
  their cached values until they notice a change; the mode therefore refuses
  instead of depending on undocumented invalidation. Two sessions on PostgreSQL
  18 were used to prove both halves: after a bare `setval` a cached backend still
  returned `2` from its block, while the `ALTER`-based step made a blocked
  `nextval` resume above the reconciled value. The refusal is a snapshot taken
  before the lock, so a cache changed by an uncoordinated external writer
  mid-run is outside the supported mode, which requires a **quiescent
  destination**. The cache check runs before the value is compared, so a
  `CACHE` greater than 1 is `reconciliation_required` even when the sequence is
  already ahead of the imported maximum: "ahead" is not proof of safety while a
  backend can hold an unobserved block. The lock also requires **ownership** of
  the identity sequence. A role that can insert but does not own the sequence
  still imports the rows, but the run reports `reconciliation_required` and the
  sequence is left untouched. If the conditions cannot be established, the mode
  fails closed instead of guessing.
- An unreconciled sequence is never reported as success: the JSON envelope is
  `ok: false` with `error.code = "sequence_reconciliation_required"`, `data`
  still carries the committed run, and the CLI exits 6 (distinct from the
  documented busy code 9).
- The run table is migrated additively: an S1 or S2-a table gains
  `sequence_status` under the DDL advisory lock, existing runs are preserved and
  backfilled to `not_required` (those milestones did not import identity
  columns), new runs default to `pending`, and any other missing column is an
  incompatible schema and is refused.
- Plans are format version 3. The plan carries an authorization envelope: an
  HMAC over the content, the issuance time, the expiry, the run id and the key
  id, produced with a key derived from the mask key under a separate domain.
  Editing any field, including re-dating both timestamps, invalidates the MAC
  even when the public fingerprint is recomputed. The expiry is bounded to 60
  minutes and the issuance may not be more than 60 seconds ahead of the executor
  clock. A version 1 or 2 plan has no authorization and is refused.
- Database driver errors expose only the SQLSTATE on both the source and the
  destination path; the server message is withheld.

Within S2-c:

- Root table filters support inclusive `gte` (`>=`) and `lte` (`<=`) bounds in
  addition to `eq` and `in`, up to 8 predicates per root combined with `AND`.
  Strict `gt` and `lt` are not accepted.
- Range bounds are permitted only on catalog-verified ordered PostgreSQL types:
  `smallint` (`int2`), `integer` (`int4`), `bigint` (`int8`), `numeric` /
  `decimal` (parsed and canonicalized as exact base-10 decimals without
  floating-point math, up to 1000 fractional digits, collapsing `-0.0` to `0`
  and stripping insignificant trailing fractional zeros), `date` (`YYYY-MM-DD`),
  `timestamp without time zone` (`YYYY-MM-DD HH:MM:SS[.ffffff]` with no offset),
  and `timestamp with time zone` (RFC 3339 with a mandatory numeric offset or
  `Z`, normalized to UTC microsecond form; zone names are refused). Unordered
  or collation-dependent types (`text`, `uuid`, `boolean`, `json`/`jsonb`,
  `bytea`, arrays, enums, ranges, domains) are refused with `invalid_filter`.
- Each predicate is canonicalized as a collision-free JSON array
  `[column, op_rank, pg_type_name, canonical_value]`, sorted and deduplicated
  before computing the keyed `filter_digest`. The digest is bound into
  `spec_hash`, `plan_fingerprint` and the v3 plan MAC, and the value-free plan
  records `filter_types` alongside `filter_columns` and `filter_ops` with
  `contains_values: false`.
- Range extraction queries bind canonical strings as parameters with explicit
  PostgreSQL casts (`$n::smallint`, `$n::integer`, `$n::bigint`, `$n::numeric`,
  `$n::date`, `$n::timestamp without time zone`, `$n::timestamptz`). No filter
  literal is ever interpolated into SQL text or written to the plan file.

The plan carries the foreign-key graph, the requested depth, the import order,
the canonical masking domains and the bounds, and all of them are bound to the
plan fingerprint. A closure that a depth or a limit cannot complete is marked
incomplete and refused, never imported partially.

## What is not implemented

- Child traversal (only parent direction is supported).
- Safe Sync for non-MariaDB MySQL.
- Batch import.
- Generated column values are not imported; they are recomputed by the
  destination. Identity regeneration and remapping are not implemented, and
  identity preservation is restricted to the single-column integer mode
  described above.
- Arbitrary transformation scripts.
- `deterministic_phone`, `date_shift`, `amount_jitter` and `fixed` masking
  strategies. A policy that selects one is refused.

This page describes the boundaries the implementation enforces. It is not a
promise of completeness beyond them.

## How it will be available

Safe Sync will be built in the Rust core first and exposed through the CLI, so it
can be tested without the desktop. The desktop will be a thin adapter over the same
core services, with no UI-only security decision.

The first implementation step is deliberately small: one table, no foreign keys,
two isolated PostgreSQL clusters, and a mandatory masking policy with an explicit
rule for every column. Relationship traversal and cyclic foreign keys come after
that spine is proven.

## Related

- [Security policy](/security/policy)
- [Destination verification](/security/destination-verification)
- [Limitations](/security/limitations)
- [Roadmap](/roadmap)
