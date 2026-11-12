<?php

namespace ProgrammingLive\LaravelSimpleBackup\Commands;

use Exception;
use Illuminate\Console\Command;
use ProgrammingLive\LaravelSimpleBackup\BackupManager;

class BackupDatabaseCommand extends Command
{
    /**
     * The name and signature of the console command.
     *
     * @var string
     */
    protected $signature = 'backup:database {--connection= : The database connection to backup}';

    /**
     * The console command description.
     *
     * @var string
     */
    protected $description = 'Backup the database (MySQL or PostgreSQL)';

    /**
     * Execute the console command.
     */
    public function handle(): int
    {
        try {
            $connection = $this->option('connection');
            
            $this->info('Starting database backup...');
            
            $filepath = BackupManager::backup($connection);
            
            $this->info("✓ Database backup completed successfully!");
            $this->info("Backup file: {$filepath}");
            
            return self::SUCCESS;
        } catch (Exception $e) {
            $this->error("✗ Backup failed: " . $e->getMessage());
            return self::FAILURE;
        }
    }
}
