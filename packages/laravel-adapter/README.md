# Laravel Simple Backup

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![PHP Version](https://img.shields.io/badge/php-%3E%3D8.1-8892BF.svg)](https://php.net/)
[![Laravel Version](https://img.shields.io/badge/laravel-%3E%3D9.0-FF2D20.svg)](https://laravel.com/)

A simple Laravel package for backing up MySQL and PostgreSQL databases.

**Author:** Mahatma Mahardhika <mahatma.mahardhika@programinglive.com>

## Features

- 🗄️ Support for MySQL and PostgreSQL databases
- 📦 Easy to install and use
- 🎯 Simple Artisan command for backups
- 💾 Automatic backup file management
- 🔧 Configurable backup settings

## Installation

```bash
composer require programinglive/laravel-simple-backup
```

## Configuration

Publish the configuration file:

```bash
php artisan vendor:publish --provider="ProgrammingLive\LaravelSimpleBackup\LaravelSimpleBackupServiceProvider" --tag="backup-config"
```

This will create a `config/backup.php` file where you can configure your backup settings.

## Usage

### Using Artisan Command

Backup the default database connection:

```bash
php artisan backup:database
```

Backup a specific database connection:

```bash
php artisan backup:database --connection=mysql
```

Or for PostgreSQL:

```bash
php artisan backup:database --connection=pgsql
```

### Using the Backup Manager Class

```php
use ProgrammingLive\LaravelSimpleBackup\BackupManager;

// Backup the default connection
$filepath = BackupManager::backup();

// Backup a specific connection
$filepath = BackupManager::backup('mysql');

// Get all backups
$backups = BackupManager::getBackups();

// Delete a backup
BackupManager::deleteBackup('backup_mydb_2024-12-08_10-30-45.sql');

// Download a backup
$filepath = BackupManager::downloadBackup('backup_mydb_2024-12-08_10-30-45.sql');
```

### Using the Facade

```php
use ProgrammingLive\LaravelSimpleBackup\Facades\Backup;

// Backup the database
$filepath = Backup::backup();

// Get all backups
$backups = Backup::getBackups();
```

## Requirements

- PHP 8.1 or higher
- Laravel 9.0, 10.0, 11.0, or 12.0
- MySQL or PostgreSQL with command-line tools installed

### System Requirements

For MySQL backups, ensure `mysqldump` is installed:
- **Windows**: Usually included with MySQL installation
- **Linux**: `sudo apt-get install mysql-client`
- **macOS**: `brew install mysql-client`

For PostgreSQL backups, ensure `pg_dump` is installed:
- **Windows**: Usually included with PostgreSQL installation
- **Linux**: `sudo apt-get install postgresql-client`
- **macOS**: `brew install postgresql`

## Backup File Location

Backups are stored in the `storage/backups` directory by default. You can change this in the `config/backup.php` file.

## Backup File Format

Backup files are named with the following format:
```
backup_{database_name}_{YYYY-MM-DD_HH-MM-SS}.sql
```

Example:
```
backup_myapp_2024-12-08_10-30-45.sql
```

## Testing

### Running Tests

```bash
# Run all tests
vendor/bin/phpunit

# Run specific test file
vendor/bin/phpunit tests/BackupManagerTest.php

# Run with coverage
vendor/bin/phpunit --coverage-html coverage
```

### Testing in a Laravel Project

To test this package in a real Laravel project:

1. Create a new Laravel project:
   ```bash
   composer create-project laravel/laravel test-backup-app
   cd test-backup-app
   ```

2. Add the package as a local repository in `composer.json`:
   ```json
   "repositories": [
       {
           "type": "path",
           "url": "../laravel-simple-backup"
       }
   ]
   ```

3. Install the package:
   ```bash
   composer require programinglive/laravel-simple-backup:@dev
   ```

4. Configure your database in `.env` and run:
   ```bash
   php artisan backup:database
   ```

5. Check the backup file in `storage/backups/`

## Contributing

Please see [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## Security

If you discover any security-related issues, please email mahatma.mahardhika@programinglive.com instead of using the issue tracker.

## License

MIT License. See [LICENSE](LICENSE) file for details.

## Support

For issues and feature requests, please visit the [GitHub repository](https://github.com/programinglive/laravel-simple-backup).
