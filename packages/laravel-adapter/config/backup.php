<?php

return [
    /*
    |--------------------------------------------------------------------------
    | Backup Configuration
    |--------------------------------------------------------------------------
    |
    | This configuration file is used to set up the backup settings for your
    | Laravel application. You can specify which database connection to backup,
    | backup directory, and other backup-related settings.
    |
    */

    'default_connection' => env('DB_CONNECTION', 'mysql'),

    'backup_path' => storage_path('backups'),

    'keep_backups' => env('BACKUP_KEEP_DAYS', 30),
];
