<?php

namespace ProgrammingLive\LaravelSimpleBackup\Tests;

use ProgrammingLive\LaravelSimpleBackup\LaravelSimpleBackupServiceProvider;

class ServiceProviderTest extends TestCase
{
    /**
     * Test that the service provider class exists.
     */
    public function test_service_provider_class_exists(): void
    {
        $this->assertTrue(class_exists(LaravelSimpleBackupServiceProvider::class));
    }

    /**
     * Test that the service provider is loaded.
     */
    public function test_service_provider_is_loaded(): void
    {
        $loadedProviders = $this->app->getLoadedProviders();
        
        $this->assertArrayHasKey(LaravelSimpleBackupServiceProvider::class, $loadedProviders);
    }

    /**
     * Test that config is merged.
     */
    public function test_config_is_merged(): void
    {
        $this->assertNotNull(config('backup'));
    }

    /**
     * Test that backup command is registered.
     */
    public function test_backup_command_is_registered(): void
    {
        $commands = $this->app->make('Illuminate\Contracts\Console\Kernel')->all();
        
        $this->assertArrayHasKey('backup:database', $commands);
    }

    /**
     * Test config can be published.
     */
    public function test_config_can_be_published(): void
    {
        $this->artisan('vendor:publish', [
            '--provider' => LaravelSimpleBackupServiceProvider::class,
            '--tag' => 'backup-config',
        ])->assertSuccessful();
    }
}
