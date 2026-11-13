# Troubleshooting

## The CLI cannot find a connection

**Symptom:** `connections test <id>` exits 7.

**Cause:** the id does not exist in the store the CLI is reading.

**Fix:** run `dbpinball connections list` and use the exact id. If you use
`--config-dir`, make sure it points at the same directory the profile was
created in.

## The credential is missing

**Symptom:** exit code 3, or the desktop asks for a password.

**Cause:** the keyring entry for the profile id is missing. This can happen after
a migration, a keyring reset, or a crash between a credential delete and a
metadata write.

**Fix:** set the password again:

```bash
dbpinball connections set-password <id>
```

The profile itself is not lost.

## The keyring is unavailable

**Symptom:** exit code 3 with a message about the keyring.

**Cause:** no Secret Service provider on Linux, a locked keychain, or a
permission problem.

**Fix:** unlock the keyring, or use `--credential-source env` with
`DBPINBALL_PASSWORD` for a one-off command. The environment source cannot store
or delete credentials.

## The store is busy

**Symptom:** exit code 9.

**Cause:** another process holds the store lock. The lock is held during a write,
including the keyring call.

**Fix:** retry. If the holder is a dead process, the lock is recovered
automatically on the next attempt. If the holder is alive but hung, the lock is
released when that process exits.

## The store changed since it was read

**Symptom:** exit code 8, or the desktop reports a conflict.

**Cause:** another process wrote the store after you read it.

**Fix:** reload and retry. The desktop reloads the list and asks you to repeat
the action.

## The desktop shows a migration warning

**Symptom:** a persistent notice about unresolved migration items.

**Cause:** the last migration skipped unreadable records or found id conflicts.

**Fix:** review the profiles. Conflicting records were not overwritten; the
existing shared profile was kept. Unreadable records were left in `localStorage`
and can be inspected there.

## The desktop cannot reach the store

**Symptom:** a "Connection store unavailable" banner.

**Cause:** the configuration directory is not writable, or the store file is not
valid JSON.

**Fix:** check permissions on the configuration directory. If the file is
corrupt, move it aside; the desktop will start with an empty store and the old
`localStorage` profiles remain available for rollback.

## A connection test fails

**Symptom:** the desktop shows "Unable to resolve host" or a connection error.

**Cause:** the host is unreachable, the port is closed, the credentials are
wrong, or the server requires TLS that is not configured.

**Fix:** verify the host and port from the same machine, confirm the database
accepts the user, and check whether the server requires SSL. The `useSsl` field
is stored but is not currently applied to connections.

## Related

- [Error behaviour reference](/reference/errors)
- [Developer troubleshooting](/reference/troubleshooting)
