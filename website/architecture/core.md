# Rust core

`dbpinball-core` is the headless domain. It has no Tauri, no CLI parser and no
terminal I/O.

## Modules

| Module | Responsibility |
| --- | --- |
| `domain::connection` | `DbConnection`, `DbEngine`, `TargetClass` |
| `domain::error` | `CoreError`, `ErrorCode`, redaction |
| `store::connection_store` | The versioned store, mutations, import, migration status |
| `store::lock` | Cross-process lock and stale recovery |
| `store::paths` | The shared configuration directory |
| `store::record` | `StoreFile`, `ImportReport`, `InvalidRecord`, `ConflictRecord` |
| `adapters::keyring` | `KeyringCredentialProvider` |
| `ports::credentials` | The `CredentialProvider` trait and `Secret` |
| `services::inspect` | Connection test, database and table listing |

## The connection model

```rust
pub struct DbConnection {
    pub id: String,
    pub name: String,
    pub engine: DbEngine,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub use_ssl: bool,
    pub target: TargetClass,
}
```

`target` is a user hint (`production`, `staging`, `local`, `unknown`). It is
metadata, not proof, and it is never used on its own to decide that a database is
safe to write to.

## Error model

`CoreError` carries a stable `ErrorCode` and a message. Every message passes
through `redact`, which removes `user:password@` from URL-shaped text. The CLI
maps the code to a documented exit code; the desktop maps it to a safe message.

## Credential port

The core does not know how a password is stored. It depends on the
`CredentialProvider` trait:

```rust
pub trait CredentialProvider: Send + Sync {
    fn get(&self, connection_id: &str) -> Result<Secret, CoreError>;
    fn set(&self, connection_id: &str, secret: &Secret) -> Result<(), CoreError>;
    fn delete(&self, connection_id: &str) -> Result<(), CoreError>;
}
```

The keyring adapter implements it for the desktop and the CLI. The CLI also has
an environment adapter that can only read.

## Related

- [Connection store](/architecture/connection-store)
- [Adapter boundaries](/architecture/adapter-boundaries)
