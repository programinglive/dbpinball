# Concurrent mutation safety

The desktop and the CLI can write the store at the same time. Two mechanisms keep
them from losing each other's changes.

## The cross-process lock

Every mutation takes `connections.lock` with an atomic exclusive create.

- **Bounded wait:** 2 s, then `ErrorCode::Busy` (exit 9).
- **Stale recovery:** only a confirmed-dead owner is recovered.
- **Live owner:** never displaced, regardless of age.
- **Unknown liveness:** treated as live (fail closed).
- **Concurrent recovery:** a recovery mutex makes re-check-then-remove one
  critical section.

## The revision

`StoreFile.revision` increments on every write. A caller that supplies a stale
`expectedRevision` gets `ErrorCode::Conflict` (exit 8) and must reload.

## Credential coordination

Create, update and delete run the revision check, the keyring call and the
metadata write in one critical section. A stale revision therefore fails before
the keyring is touched.

## What is not claimed

- Cross-system atomicity between the JSON store and the keyring.
- A bounded time for a hung keyring call. The lock is held during the call; the
  wait is bounded by the process lifetime.

## Tests

- `two_processes_do_not_lose_each_others_import`
- `a_live_lock_owner_causes_a_busy_exit`
- `a_dead_lock_owner_is_recovered`
- `a_lock_from_another_host_is_not_stolen`
- `a_stale_revision_does_not_touch_the_credential`
- `a_concurrent_writer_is_serialized_against_delete`

## Related

- [Revision consistency](/architecture/revision-consistency)
- [Credential lifecycle](/architecture/credential-lifecycle)
