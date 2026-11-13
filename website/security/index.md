# Security

This section describes how DBPinball handles credentials, what it guarantees, and
where the guarantees stop.

## The short version

- Passwords live in the OS keyring, never in the connection store.
- There is no plaintext fallback.
- Error messages are redacted.
- Concurrent mutations are serialized by a cross-process lock and a revision.
- The JSON store and the keyring are separate systems, so cross-system
  crash-atomicity is not claimed.

## Read next

- [Credential storage](/security/credential-storage)
- [No plaintext fallback](/security/no-plaintext-fallback)
- [OS keyring integration](/security/keyring)
- [Redaction guarantees](/security/redaction)
- [Concurrent mutation safety](/security/concurrency)
- [Limitations](/security/limitations)
- [Platforms](/security/platforms)
