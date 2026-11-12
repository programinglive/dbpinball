<?php

namespace ProgrammingLive\LaravelSimpleBackup\Tests;

use Orchestra\Testbench\TestCase as BaseTestCase;
use ProgrammingLive\LaravelSimpleBackup\LaravelSimpleBackupServiceProvider;

abstract class TestCase extends BaseTestCase
{
    /**
     * Get package providers.
     *
     * @param \Illuminate\Foundation\Application $app
     * @return array<int, class-string>
     */
    protected function getPackageProviders($app): array
    {
        return [
            LaravelSimpleBackupServiceProvider::class,
        ];
    }

    /**
     * Define environment setup.
     *
     * @param \Illuminate\Foundation\Application $app
     * @return void
     */
    protected function defineEnvironment($app): void
    {
        // Setup default database to use sqlite :memory:
        $app['config']->set('database.default', 'testing');
        $app['config']->set('database.connections.testing', [
            'driver' => 'sqlite',
            'database' => ':memory:',
            'prefix' => '',
        ]);

        // Setup MySQL test connection (for testing backup logic)
        $app['config']->set('database.connections.mysql_test', [
            'driver' => 'mysql',
            'host' => env('DB_HOST', '127.0.0.1'),
            'port' => env('DB_PORT', '3306'),
            'database' => env('DB_DATABASE', 'test_db'),
            'username' => env('DB_USERNAME', 'root'),
            'password' => env('DB_PASSWORD', ''),
        ]);

        // Setup PostgreSQL test connection
        $app['config']->set('database.connections.pgsql_test', [
            'driver' => 'pgsql',
            'host' => env('DB_HOST', '127.0.0.1'),
            'port' => env('DB_PORT', '5432'),
            'database' => env('DB_DATABASE', 'test_db'),
            'username' => env('DB_USERNAME', 'postgres'),
            'password' => env('DB_PASSWORD', ''),
        ]);

        // Setup backup config
        $app['config']->set('backup.default_connection', 'testing');
        $app['config']->set('backup.backup_path', storage_path('backups'));
        $app['config']->set('backup.keep_backups', 30);
    }

    /**
     * Get the backup directory path.
     */
    protected function getBackupPath(): string
    {
        return storage_path('backups');
    }

    /**
     * Clean up backup directory.
     */
    protected function cleanupBackups(): void
    {
        $backupPath = $this->getBackupPath();
        if (is_dir($backupPath)) {
            $files = glob($backupPath . '/*.sql');
            foreach ($files as $file) {
                if (is_file($file)) {
                    unlink($file);
                }
            }
        }
    }

    /**
     * Setup the test environment.
     */
    protected function setUp(): void
    {
        parent::setUp();
        $this->cleanupBackups();
    }

    /**
     * Tear down the test environment.
     */
    protected function tearDown(): void
    {
        $this->cleanupBackups();
        parent::tearDown();
    }
}
