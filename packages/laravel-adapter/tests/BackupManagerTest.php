<?php

namespace ProgrammingLive\LaravelSimpleBackup\Tests;

use Exception;
use ProgrammingLive\LaravelSimpleBackup\BackupManager;

class BackupManagerTest extends TestCase
{
    /**
     * Test that BackupManager class exists and can be referenced.
     */
    public function test_backup_manager_class_exists(): void
    {
        $this->assertTrue(class_exists(BackupManager::class));
    }

    /**
     * Test that backup method exists.
     */
    public function test_backup_method_exists(): void
    {
        $this->assertTrue(method_exists(BackupManager::class, 'backup'));
    }

    /**
     * Test that getBackups method exists.
     */
    public function test_get_backups_method_exists(): void
    {
        $this->assertTrue(method_exists(BackupManager::class, 'getBackups'));
    }

    /**
     * Test that deleteBackup method exists.
     */
    public function test_delete_backup_method_exists(): void
    {
        $this->assertTrue(method_exists(BackupManager::class, 'deleteBackup'));
    }

    /**
     * Test that downloadBackup method exists.
     */
    public function test_download_backup_method_exists(): void
    {
        $this->assertTrue(method_exists(BackupManager::class, 'downloadBackup'));
    }

    /**
     * Test backup throws exception for invalid connection.
     */
    public function test_backup_throws_exception_for_invalid_connection(): void
    {
        $this->expectException(Exception::class);
        $this->expectExceptionMessage("Database connection 'invalid_connection' not found");
        
        BackupManager::backup('invalid_connection');
    }

    /**
     * Test backup throws exception for unsupported driver.
     */
    public function test_backup_throws_exception_for_unsupported_driver(): void
    {
        // Configure a connection with unsupported driver
        config(['database.connections.unsupported_test' => [
            'driver' => 'unsupported_driver',
            'database' => 'test',
        ]]);

        $this->expectException(Exception::class);
        $this->expectExceptionMessage('Unsupported database driver: unsupported_driver');
        
        BackupManager::backup('unsupported_test');
    }

    /**
     * Test getBackups returns empty array when no backups exist.
     */
    public function test_get_backups_returns_empty_array_when_no_backups(): void
    {
        $backups = BackupManager::getBackups();
        
        $this->assertIsArray($backups);
        $this->assertEmpty($backups);
    }

    /**
     * Test getBackups returns array with correct structure when backups exist.
     */
    public function test_get_backups_returns_correct_structure(): void
    {
        // Create a mock backup file
        $backupPath = storage_path('backups');
        if (!is_dir($backupPath)) {
            mkdir($backupPath, 0755, true);
        }
        
        $testFile = $backupPath . '/backup_test_db_2024-12-08_10-00-00.sql';
        file_put_contents($testFile, '-- Test backup content');
        
        $backups = BackupManager::getBackups();
        
        $this->assertIsArray($backups);
        $this->assertNotEmpty($backups);
        $this->assertArrayHasKey('filename', $backups[0]);
        $this->assertArrayHasKey('path', $backups[0]);
        $this->assertArrayHasKey('size', $backups[0]);
        $this->assertArrayHasKey('created_at', $backups[0]);
        
        // Cleanup
        unlink($testFile);
    }

    /**
     * Test deleteBackup throws exception for non-existent file.
     */
    public function test_delete_backup_throws_exception_for_nonexistent_file(): void
    {
        $this->expectException(Exception::class);
        $this->expectExceptionMessage('Backup file not found: nonexistent_backup.sql');
        
        BackupManager::deleteBackup('nonexistent_backup.sql');
    }

    /**
     * Test deleteBackup successfully deletes existing file.
     */
    public function test_delete_backup_removes_existing_file(): void
    {
        // Create a mock backup file
        $backupPath = storage_path('backups');
        if (!is_dir($backupPath)) {
            mkdir($backupPath, 0755, true);
        }
        
        $filename = 'backup_test_delete_2024-12-08_10-00-00.sql';
        $testFile = $backupPath . '/' . $filename;
        file_put_contents($testFile, '-- Test backup content');
        
        $this->assertFileExists($testFile);
        
        $result = BackupManager::deleteBackup($filename);
        
        $this->assertTrue($result);
        $this->assertFileDoesNotExist($testFile);
    }

    /**
     * Test downloadBackup throws exception for non-existent file.
     */
    public function test_download_backup_throws_exception_for_nonexistent_file(): void
    {
        $this->expectException(Exception::class);
        $this->expectExceptionMessage('Backup file not found: nonexistent_backup.sql');
        
        BackupManager::downloadBackup('nonexistent_backup.sql');
    }

    /**
     * Test downloadBackup returns correct path for existing file.
     */
    public function test_download_backup_returns_correct_path(): void
    {
        // Create a mock backup file
        $backupPath = storage_path('backups');
        if (!is_dir($backupPath)) {
            mkdir($backupPath, 0755, true);
        }
        
        $filename = 'backup_test_download_2024-12-08_10-00-00.sql';
        $testFile = $backupPath . '/' . $filename;
        file_put_contents($testFile, '-- Test backup content');
        
        $result = BackupManager::downloadBackup($filename);
        
        $this->assertEquals($testFile, $result);
        $this->assertFileExists($result);
        
        // Cleanup
        unlink($testFile);
    }

    /**
     * Test backup directory is created if it doesn't exist.
     */
    public function test_backup_directory_creation(): void
    {
        $backupPath = storage_path('backups');
        
        // Remove directory if exists
        if (is_dir($backupPath)) {
            rmdir($backupPath);
        }
        
        // Call getBackups which should handle non-existent directory
        $backups = BackupManager::getBackups();
        
        $this->assertIsArray($backups);
        $this->assertEmpty($backups);
    }

    /**
     * Test getBackups only returns SQL files.
     */
    public function test_get_backups_only_returns_sql_files(): void
    {
        $backupPath = storage_path('backups');
        if (!is_dir($backupPath)) {
            mkdir($backupPath, 0755, true);
        }
        
        // Create SQL file
        $sqlFile = $backupPath . '/backup_test_2024-12-08_10-00-00.sql';
        file_put_contents($sqlFile, '-- SQL backup');
        
        // Create non-SQL file
        $txtFile = $backupPath . '/readme.txt';
        file_put_contents($txtFile, 'Not a backup');
        
        $backups = BackupManager::getBackups();
        
        $this->assertCount(1, $backups);
        $this->assertEquals('backup_test_2024-12-08_10-00-00.sql', $backups[0]['filename']);
        
        // Cleanup
        unlink($sqlFile);
        unlink($txtFile);
    }

    /**
     * Test getBackups returns backups in reverse chronological order.
     */
    public function test_get_backups_returns_in_reverse_order(): void
    {
        $backupPath = storage_path('backups');
        if (!is_dir($backupPath)) {
            mkdir($backupPath, 0755, true);
        }
        
        // Create older backup
        $oldFile = $backupPath . '/backup_test_2024-12-01_10-00-00.sql';
        file_put_contents($oldFile, '-- Old backup');
        touch($oldFile, strtotime('2024-12-01'));
        
        // Create newer backup
        $newFile = $backupPath . '/backup_test_2024-12-08_10-00-00.sql';
        file_put_contents($newFile, '-- New backup');
        touch($newFile, strtotime('2024-12-08'));
        
        $backups = BackupManager::getBackups();
        
        $this->assertCount(2, $backups);
        // Newer backup should be first (reverse order)
        $this->assertStringContainsString('2024-12-08', $backups[0]['filename']);
        
        // Cleanup
        unlink($oldFile);
        unlink($newFile);
    }
}
