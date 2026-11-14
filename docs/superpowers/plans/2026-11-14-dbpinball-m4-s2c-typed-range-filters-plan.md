# DBPinball M4 S2-c - PostgreSQL Typed Range Filters and Selection Controls

> **Status:** planning only. S2-c is not implemented in this milestone. This
> document is the implementation-ready scope for the next slice.

**Goal:** Let a Safe Sync selection bound a root table by an inclusive range on a
typed PostgreSQL column, so a developer can export "orders created since Monday"
without exporting the whole table, while every predicate stays typed, bounded,
canonical and bound into the authenticated plan.

**Depends on:** M4 S2-b (plan v3 authorization envelope, typed NULL handling,
generated-expression fingerprinting, restricted sequence reconciliation).

**Non-goals:** no `gt`/`lt` in this slice, no OR/NOT, no subqueries, no joins in
the filter, no pattern matching (`LIKE`), no full-text search, no cross-column
comparisons, no filter expressions over masked values, no destination-side
filtering, no S2-d work.

## 1. Supported operators

The operator set stays closed. `SafeSyncSpec::FilterOp` is the single place that
decides what a spec may contain.

| Operator | SQL | Status in S2-c |
| --- | --- | --- |
| `eq` | `col = $1` | already supported; keep unchanged |
| `in` | `col = ANY($1)` | already supported; keep unchanged |
| `gte` | `col >= $1` | **new** |
| `lte` | `col <= $1` | **new** |
| `gt` | `col > $1` | evaluated, **deferred** |
| `lt` | `col < $1` | evaluated, **deferred** |

`gte` and `lte` are inclusive. A half-open window is expressible today as
`gte` + `lte`; `gt`/`lt` add nothing that a caller cannot express with one extra
unit of the column's type (except for continuous types, where `gt`/`lt` are
genuinely different). Because no current use case needs them, they are not added:
a later slice can add them without changing the canonical encoding, and adding
them now would enlarge the accepted surface without a test that justifies it.

An unknown operator, or a range operator on a column whose type has no total
order, is `invalid_filter`.

## 2. Typed values

A filter value is validated against `pg_catalog` metadata for the column before
any SQL is built. The column's `atttypid` selects one of the supported classes;
anything else is `unsupported_type` (existing code path), not a best-effort cast.

| PostgreSQL type | Accepted literal | Canonical form | Refusal |
| --- | --- | --- | --- |
| `smallint`, `integer`, `bigint` | optional sign, decimal digits only | `i64::to_string()` | out of range for the column type, `+` sign, leading zeros (`007`), any decimal point |
| `numeric`, `decimal` | optional sign, digits, optional `.` digits; no exponent | normalized decimal string (see below) | exponent notation, `NaN`, `Infinity`, more than 1000 fractional digits |
| `date` | `YYYY-MM-DD` | same | any time part, any offset, invalid calendar date |
| `timestamp without time zone` | `YYYY-MM-DD HH:MM[:SS[.ffffff]]` | same, space separator | any offset or `Z` (ambiguous), invalid date/time |
| `timestamp with time zone` | as above **plus a required offset** (`Z`, `+HH:MM`, `-HH:MM`) | UTC, microsecond precision | missing offset, zone name (`Europe/Paris`), invalid offset |
| `boolean` | n/a for range | n/a | `gte`/`lte` on boolean is `invalid_filter` |
| `text`, `uuid`, `json*`, arrays, enums, domains, custom | n/a for range | n/a | `invalid_filter` (range on a non-ordered or unsupported type) |

**NUMERIC normalization.** Decimals are never converted to `f64`. The literal is
parsed as sign + integer digits + fraction digits, trailing fraction zeros are
removed, and the value is re-rendered as a plain decimal string (`1.10` -> `1.1`,
`-0.0` -> `0`). The digest is computed over this canonical string, and the SQL
value is bound as that string with an explicit `::numeric` cast, so the database
receives the exact decimal the digest covers. `1.10` and `1.1` are therefore the
same predicate, which is correct for a numeric column.

**Integers** are parsed into `i64` and range-checked against the column type
(`int2`/`int4`/`int8`) before binding. No float, no truncation.

**NULL.** A predicate value cannot be `null` for `gte`/`lte`: NULL has no order,
so `null` is `invalid_filter`. A row whose column is NULL never satisfies a range
predicate; that is SQL three-valued logic and is documented, not emulated. The
typed-NULL preservation from S2-b is unchanged.

**Unsupported and custom types** fail before extraction, with the existing
`unsupported_type` / `invalid_filter` codes. Domains are resolved to their base
type; if the base type is not in the table above, the predicate is refused.

## 3. Timezone handling

- `timestamptz` literals **must** carry an explicit numeric offset or `Z`. A
  literal without an offset is refused, because the result would depend on the
  server's `TimeZone` setting.
- Zone **names** are not accepted. A name makes the literal depend on the
  timezone database version and on daylight-saving rules; a numeric offset is
  unambiguous. A caller that wants "midnight in Paris" computes the offset in the
  calendar it already trusts.
- The canonical form is UTC with microsecond precision, rendered as
  `YYYY-MM-DDTHH:MM:SS.ffffffZ` (trailing zeros trimmed to the shortest exact
  form). The plan and the digest store this form, not the original literal.
- `timestamp without time zone` literals must **not** carry an offset. They are
  compared against the stored wall-clock values as-is; no conversion happens.
- Daylight-saving transitions cannot create a duplicated or skipped instant,
  because only absolute offsets are accepted.
- Both bounds are inclusive (`>=`, `<=`). A value exactly on the boundary is
  selected. Fractional seconds beyond microseconds are refused rather than
  rounded, so a boundary can never move silently.

## 4. Predicate composition

- Multiple predicates on one root are combined with **AND**.
- A predicate is `{ column, op, typed value }`. The canonical encoding is
  `column_ordinal || 0x1f || operator_rank || 0x1f || canonical_value`, where
  `column_ordinal` is the column's `attnum` (not its name, so a rename does not
  silently reorder) and `operator_rank` is a fixed rank (`eq`=1, `in`=2, `gte`=3,
  `lte`=4).
- Predicates are sorted by that encoding before hashing, so incidental JSON key
  order, YAML sequence order and duplicate predicates cannot change the
  resulting digest. Duplicate predicates collapse to one.
- `in` keeps its existing bounded list (the existing limit) and is treated as one
  predicate; the sorted element list is part of its canonical value.
- A hard cap on predicates per root (proposed: 8) keeps the generated SQL and the
  digest bounded. The cap is part of the limits and is reported as
  `limit_exceeded`.

## 5. Privacy and authorization

- The plan stays **value-free**. It records, per predicate: column name, operator,
  PostgreSQL type name, and the SHA-256 of the canonical typed value. It never
  records the literal.
- The filter digest is computed over the canonical typed value, not over the
  original text, so `1.10` and `1.1` cannot produce two different approved plans
  for the same numeric predicate.
- The predicate digest list is folded into `spec_hash`, and `spec_hash` is already
  folded into `plan_fingerprint`. A modified predicate (column, operator, type or
  value) therefore changes the fingerprint, and the plan v3 HMAC over the content
  no longer verifies: the plan is refused before any connection is opened.
- The existing authorization envelope is unchanged: content + `created_at_ms` +
  `expires_at_ms` + `run_id` + key id, HMAC with the domain-separated plan key.
- No plaintext filter value reaches a log, an audit record, an error message or a
  CLI argument. Error details carry the predicate index and the digest prefix.
- Every value is a bound parameter with an explicit cast (`$1::numeric`,
  `$1::timestamptz`, ...). Column names are validated against the discovered
  graph and emitted with `quote_ident`; no identifier comes from the spec text.
- The source session keeps `default_transaction_read_only = on`, the existing
  `statement_timeout`, and the existing source-privilege refusal. A filter cannot
  introduce a write path.
- Row-level security still applies. A filter is a selection convenience, never an
  authorization boundary.

## 6. Acceptance tests

Real PostgreSQL, disposable clusters, synthetic data, `--features integration`.
Skipped tests are not acceptance.

1. `gte` returns rows equal to and above the bound (inclusive lower).
2. `lte` returns rows equal to and below the bound (inclusive upper).
3. `gte` + `lte` on the same column returns the closed window, including both
   boundary rows.
4. A bound outside the data returns the correct empty or full set, not an error.
5. An integer bound is refused when it overflows the column type (`invalid_filter`).
6. `numeric` treats `1.10` and `1.1` as the same predicate; a `1.10000000000000001`
   bound is not silently rounded to `1.1`.
7. `date` accepts `YYYY-MM-DD` and refuses a time part.
8. `timestamp` refuses an offset; `timestamptz` refuses a missing offset.
9. `timestamptz` normalizes `2026-01-01T07:00:00+07:00` and
   `2026-01-01T00:00:00Z` to the same digest and returns the same rows.
10. A NULL column value is never returned by a range predicate (three-valued
    logic), and the typed NULL round-trip from S2-b still holds.
11. `gte` on a `text`, `boolean`, `uuid` or custom type is refused before
    extraction.
12. Predicate order, duplicate predicates and JSON key order do not change
    `spec_hash` or the plan fingerprint.
13. Changing one bound changes `spec_hash`, and an old plan is refused with the
    existing schema/spec-drift code before any row is read.
14. An edited plan file cannot restore authorization: the v3 HMAC fails.
15. The generated SQL contains only bound parameters; a canary literal never
    appears in the statement, the plan, the audit record or an error message.
16. The predicate cap is enforced (`limit_exceeded`), and the row/byte/depth/time
    limits still apply to the filtered result.
17. The source session remains read-only during a ranged extraction.
18. S1, S2-a and S2-b suites still pass unchanged.

## 7. Implementation outline (TDD)

1. Extend `FilterOp` with `gte` and `lte`; add the closed-set validation and a
   unit test that `gt`/`lt` are still rejected.
2. Add `TypedValue` with the parsers from section 2 and unit tests for every
   refusal row, including numeric canonicalization without floats.
3. Add the canonical predicate encoding and digest with unit tests for order
   independence, duplicates and `1.10`/`1.1`.
4. Extend `planner` to emit `$n` parameters and explicit casts; assert the SQL
   text contains no literal from the spec.
5. Fold the digest list into `spec_hash`; assert a changed bound invalidates the
   v3 plan MAC before a connection is opened.
6. Add the acceptance tests from section 6 against two disposable clusters.
7. Update `website/security/safe-sync-design.md` and the architecture spec, run
   the website link/contrast/layout checks, and keep `docs/cli.md` in sync.

**Definition of done for S2-c:** the closed operator set, every typed-value
refusal, the inclusive boundaries, the timezone rules, the canonical digest, the
plan-binding test and the source read-only test all pass against real PostgreSQL;
the plan remains value-free; no S2-b guarantee is weakened; CI is green on the
self-hosted `bw-tools` runner.

## 8. Risks and open questions

- **Filter selectivity.** A range that matches most of a table is still bounded by
  the row/byte limits; the plan already refuses when the limit is exceeded. No
  new sampling or estimation is proposed.
- **Collation.** Range filters on `text` would depend on the database collation.
  That is one reason text ranges are refused in this slice.
- **`in` with NULL.** Existing behavior is unchanged; a NULL element never
  matches, and the typed NULL round-trip is preserved.
- **Sequence of predicates on different columns.** AND across columns is
  supported; the optimizer decides the order, and the plan states the predicate
  set, not the chosen plan.
- **Future `gt`/`lt`.** The canonical encoding reserves operator ranks 5 and 6, so
  they can be added later without changing the digest of existing predicates.
