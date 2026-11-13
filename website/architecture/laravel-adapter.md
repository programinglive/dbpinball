# Laravel adapter

`packages/laravel-adapter` is a separate PHP package in this repository. It is
not part of the Rust core and it does not share the connection store.

## What it provides

- the `backup:database` Artisan command;
- a `BackupManager` for MySQL and PostgreSQL dumps.

## What it does not provide

- retention pruning;
- cloud storage drivers;
- Safe Sync.

## Relationship to the desktop

The adapter is a server-side package. It does not read the desktop connection
store and it does not use the OS keyring. It is configured through Laravel's own
configuration.

## Testing

The package has its own `composer.json` and PHPUnit suite. CI runs
`composer validate`, `composer audit` and `phpunit` in the
`PHP / Laravel Adapter` job.

## Related

- [Adapter boundaries](/architecture/adapter-boundaries)
- [CI checks](/development/ci)
