# Revision consistency

The store is a JSON file that two processes can write. Two mechanisms keep them
from losing each other's changes: a cross-process lock and a monotonic revision.

## The lock

`connections.lock` is created with `create_new`, which the operating system
guarantees only one process can do. The holder records its PID, host and start
time.

| Property | Behaviour |
| --- | --- |
| Acquisition | Atomic exclusive create |
| Bounded wait | 2 s, 20 ms poll, then `ErrorCode::Busy` (exit 9) |
| Stale recovery | Only a **confirmed-dead** owner is recovered |
| Live owner | Never displaced, regardless of age |
| Unknown liveness | Treated as live (fail closed) |
| Concurrent recovery | A recovery mutex makes re-check-then-remove one critical section |

Liveness is checked with `ps -p` on Unix and `tasklist` on Windows. `kill -0` is
deliberately not used: it exits non-zero for both "no such process" and "not
permitted", so it cannot prove death.

## The revision

`StoreFile.revision` increments on every successful write. A caller that read
revision N and supplies it as `expectedRevision` gets `ErrorCode::Conflict`
(exit 8) if the store is now at N+1. The caller reloads and retries.

The revision is checked **inside** the lock, so it cannot be bypassed by a
concurrent writer.

## What this guarantees

- Two writers cannot both commit a change based on the same read.
- A stale desktop window cannot overwrite a CLI change.
- A stale revision cannot delete a credential for a profile that stays.

## What it does not guarantee

- Cross-system atomicity between the JSON store and the OS keyring. See
  [Credential lifecycle](/architecture/credential-lifecycle).
- A bounded time for a hung keyring call. The lock is held during the call; the
  wait is bounded by the process lifetime, not by a timeout.

## Related

- [Connection store](/architecture/connection-store)
- [Concurrent mutation safety](/security/concurrency)
