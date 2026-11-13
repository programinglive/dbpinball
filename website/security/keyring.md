# OS keyring integration

DBPinball uses the `keyring` crate to reach the platform credential store.

## Backends

| Platform | Backend |
| --- | --- |
| Windows | Credential Manager |
| macOS | Keychain |
| Linux | libsecret / Secret Service |

## The provider

`KeyringCredentialProvider` implements the core `CredentialProvider` trait:

```rust
fn get(&self, connection_id: &str) -> Result<Secret, CoreError>;
fn set(&self, connection_id: &str, secret: &Secret) -> Result<(), CoreError>;
fn delete(&self, connection_id: &str) -> Result<(), CoreError>;
```

`delete` treats a missing entry as success: the desired end state is "no
credential stored", and a profile being removed should not fail because the
credential was already gone.

## The environment provider

`EnvCredentialProvider` reads `DBPINBALL_PASSWORD`. Its `set` and `delete` return
a usage error, because an environment variable is not a store.

## Headless Linux

A headless Linux host may have no Secret Service provider. In that case the
keyring is unavailable and the CLI fails with exit code 3. Use
`--credential-source env` for a one-off command, or run a Secret Service provider.

## Related

- [Credential storage](/security/credential-storage)
- [Platforms](/security/platforms)
