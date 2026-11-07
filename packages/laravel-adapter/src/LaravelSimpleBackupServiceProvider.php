<?php

namespace ProgrammingLive\LaravelSimpleBackup;

use Illuminate\Support\ServiceProvider;
use ProgrammingLive\LaravelSimpleBackup\Commands\BackupDatabaseCommand;

class LaravelSimpleBackupServiceProvider extends ServiceProvider
{
    /**
     * Register services.
     */
    public function register(): void
    {
        $this->mergeConfigFrom(
            __DIR__ . '/../config/backup.php',
            'backup'
        );
    }

    /**
     * Bootstrap services.
     */
    public function boot(): void
    {
        $this->publishes([
            __DIR__ . '/../config/backup.php' => config_path('backup.php'),
        ], 'backup-config');

        if ($this->app->runningInConsole()) {
            $this->commands([
                BackupDatabaseCommand::class,
            ]);
        }
    }
}
