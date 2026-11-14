# Limitations

DBPinball is honest about what it does not guarantee. This page lists the limits
that matter for security and reliability.

## Cross-system crash atomicity

The JSON store and the OS keyring are separate persistence systems. A crash
between a credential delete and a metadata write leaves the profile without a
credential. The profile is not lost; the user re-enters the password.

A crash between a credential write and a metadata write on create leaves an
orphan credential and no profile. It is harmless and is overwritten by a later
create with the same id.

## Hung keyring

The store lock is held while the keyring is called. The call is synchronous and
cannot be cancelled, so a hung keyring blocks other writers until the process
exits. The lock's liveness-based recovery then unblocks them. This is **not** a
strictly bounded-time operation.

## PID reuse

A reused PID makes a dead lock owner look alive. The contender times out instead
of stealing the lock. This is fail-closed and safe, but it can delay recovery.

## No remote lock

The lock is a local file. Two machines sharing a configuration directory over a
network filesystem are not coordinated. The store is designed for a per-user
local configuration directory.

## Not a sandbox

DBPinball is an application, not a sandbox. It does not restrict what a user can
do with the database once connected.

## Engine-specific read-only gaps

- **MySQL**: a read-only transaction does not cover DDL, because MySQL performs
  an implicit commit first. MyISAM tables ignore transactions entirely. The
  classifier routes DDL to the write path, but the engine guarantee is narrower
  than PostgreSQL's.
- **PostgreSQL**: a read-only transaction covers DML and DDL. A function with
  external side effects can still act outside the database.
- **Both**: least-privilege credentials remain the only enforcement the engine
  cannot be talked out of.

## Enrollment concurrency

The enrollment registry takes the same cross-process lock as the connection
store. A corrupted registry is an error, never a silently empty one, so an
enrolled destination cannot appear unenrolled after a partial write.


## Not implemented

- Shared security policy and destination verification (M3).
- Safe Sync (M4 and later).
- Backup, restore and copy from the desktop UI (commands exist, pages are not
  routed).
- TLS enforcement (`useSsl` is stored but not applied).

## Related

- [Platforms](/security/platforms)
- [Roadmap](/roadmap)
