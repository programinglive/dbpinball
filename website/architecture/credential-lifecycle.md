# Credential lifecycle

A profile and its credential are two persistence systems: the JSON store and the
OS keyring. They are coordinated as one logical mutation under the store lock.

## Operations

| Operation | Order inside the critical section | Failure behaviour |
| --- | --- | --- |
| **Create** | duplicate-id check → credential write → metadata write | Duplicate id or failed credential write persists nothing |
| **Update** | revision check → profile lookup → credential write → metadata write | Stale revision or failed credential write leaves the profile unchanged |
| **Delete** | revision check → profile removal → credential delete → metadata write | Stale revision or failed credential delete leaves the profile and its credential intact |

## Guarantees

- A stale revision cannot delete the credential of a profile that remains active.
- Concurrent CLI and desktop mutations cannot invalidate each other's credentials.
- A failed keyring operation does not remove a valid profile.
- A failed metadata operation does not destroy a credential.
- No plaintext fallback is introduced.
- Profile removal never touches a database or its data.

## Crash consistency

The JSON store and the keyring are separate systems, so this is **not**
crash-atomic.

| Crash point | Result | Recovery |
| --- | --- | --- |
| After credential delete, before metadata write | Profile remains without a credential | The desktop reports "password required"; re-enter it |
| After credential write, before metadata write (create) | Orphan credential, no profile | Harmless; overwritten by a later create with the same id |

## Hung keyring

The lock is held while the keyring is called. The call is synchronous and cannot
be cancelled, so a hung keyring blocks other writers until the process exits. The
lock's liveness-based recovery then unblocks them. This is **not** a strictly
bounded-time operation.

## Related

- [Credentials and keyring](/guide/credentials)
- [Concurrent mutation safety](/security/concurrency)
- [Limitations](/security/limitations)
