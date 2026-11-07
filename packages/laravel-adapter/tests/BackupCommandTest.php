<?php

namespace ProgrammingLive\LaravelSimpleBackup\Tests;

use ProgrammingLive\LaravelSimpleBackup\Commands\BackupDatabaseCommand;

class BackupCommandTest extends TestCase
{
    /**
     * Test that the backup command class exists.
     */
    public function test_backup_command_class_exists(): void
    {
        $this->assertTrue(class_exists(BackupDatabaseCommand::class));
    }

    /**
     * Test that the command is registered.
     */
    public function test_backup_command_is_registered(): void
    {
        $this->assertTrue($this->app->make('Illuminate\Contracts\Console\Kernel')->all()['backup:database'] !== null);
    }

    /**
     * Test command signature.
     */
    public function test_backup_command_has_correct_signature(): void
    {
        $command = new BackupDatabaseCommand();
        
        $this->assertStringContainsString('backup:database', $command->getName());
    }

    /**
     * Test command description.
     */
    public function test_backup_command_has_description(): void
    {
        $command = new BackupDatabaseCommand();
        
        $this->assertNotEmpty($command->getDescription());
        $this->assertStringContainsString('Backup', $command->getDescription());
    }

    /**
     * Test command fails gracefully with invalid connection.
     */
    public function test_backup_command_fails_with_invalid_connection(): void
    {
        $this->artisan('backup:database', ['--connection' => 'invalid_connection'])
            ->assertFailed();
    }

    /**
     * Test command has connection option.
     */
    public function test_backup_command_has_connection_option(): void
    {
        $command = new BackupDatabaseCommand();
        $definition = $command->getDefinition();
        
        $this->assertTrue($definition->hasOption('connection'));
    }
}
