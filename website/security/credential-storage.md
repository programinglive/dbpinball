# Credential storage

## Where a password goes

A password is written to the OS keyring under:

- **service**: `dbpinball`
- **account**: the connection profile id

It is never written to `connections.json`, never written to `localStorage`, and
never passed as a command-line argument.

## Why the account is the profile id

The keyring entry is keyed by the profile id, so renaming a profile does not
orphan its credential. The id is stable for the life of the profile.

## What the store contains

Metadata only:

```json
{
  "id": "0f0c2f7e-...",
  "name": "Analytics",
  "engine": "postgres",
  "host": "analytics.example.com",
  "port": 5432,
  "database": "analytics",
  "username": "analyst",
  "useSsl": true,
  "target": "unknown"
}
```

A test asserts the serialized store never contains the string `password`.

## Migration

The desktop migration from the old browser `localStorage` imports metadata only.
It never reads, writes or regenerates a password. Existing keyring entries keep
working because the service and account are unchanged.

## Related

- [OS keyring integration](/security/keyring)
- [Credential lifecycle](/architecture/credential-lifecycle)
