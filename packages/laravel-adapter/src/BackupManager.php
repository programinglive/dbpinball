<?php

namespace ProgrammingLive\LaravelSimpleBackup;

use Exception;

class BackupManager
{
    /**
     * Backup the database
     */
    public static function backup(?string $connection = null): string
    {
        $connection = $connection ?? \config('database.default');
        $config = \config("database.connections.{$connection}");

        if (!$config) {
            throw new Exception("Database connection '{$connection}' not found");
        }

        $driver = $config['driver'] ?? null;

        return match ($driver) {
            'mysql' => self::backupMysql($config),
            'pgsql' => self::backupPostgres($config),
            default => throw new Exception("Unsupported database driver: {$driver}"),
        };
    }

    /**
     * Backup MySQL database
     */
    private static function backupMysql(array $config): string
    {
        $host = $config['host'] ?? 'localhost';
        $user = $config['username'] ?? 'root';
        $password = $config['password'] ?? '';
        $database = $config['database'] ?? '';
        $port = $config['port'] ?? 3306;

        $backupDir = \storage_path('backups');
        if (!is_dir($backupDir)) {
            mkdir($backupDir, 0755, true);
        }

        $filename = "backup_{$database}_" . date('Y-m-d_H-i-s') . '.sql';
        $filepath = "{$backupDir}/{$filename}";

        $command = "mysqldump --host={$host} --port={$port} --user={$user}";

        if (!empty($password)) {
            $command .= " --password={$password}";
        }

        $command .= " {$database} > \"{$filepath}\"";

        // For Windows, use different command syntax
        if (strtoupper(substr(PHP_OS, 0, 3)) === 'WIN') {
            $command = "mysqldump --host={$host} --port={$port} --user={$user}";
            if (!empty($password)) {
                $command .= " --password=\"{$password}\"";
            }
            $command .= " {$database} > \"{$filepath}\"";
        }

        $output = null;
        $returnVar = null;
        exec($command, $output, $returnVar);

        if ($returnVar !== 0) {
            throw new Exception("MySQL backup failed. Error: " . implode("\n", $output ?? []));
        }

        return $filepath;
    }

    /**
     * Backup PostgreSQL database
     */
    private static function backupPostgres(array $config): string
    {
        $host = $config['host'] ?? 'localhost';
        $user = $config['username'] ?? 'postgres';
        $password = $config['password'] ?? '';
        $database = $config['database'] ?? '';
        $port = $config['port'] ?? 5432;

        $backupDir = \storage_path('backups');
        if (!is_dir($backupDir)) {
            mkdir($backupDir, 0755, true);
        }

        $filename = "backup_{$database}_" . date('Y-m-d_H-i-s') . '.sql';
        $filepath = "{$backupDir}/{$filename}";

        // Set password environment variable for PostgreSQL
        if (!empty($password)) {
            putenv("PGPASSWORD={$password}");
        }

        $command = "pg_dump --host={$host} --port={$port} --username={$user} {$database} > \"{$filepath}\"";

        // For Windows
        if (strtoupper(substr(PHP_OS, 0, 3)) === 'WIN') {
            $command = "pg_dump --host={$host} --port={$port} --username={$user} {$database} > \"{$filepath}\"";
        }

        $output = null;
        $returnVar = null;
        exec($command, $output, $returnVar);

        if ($returnVar !== 0) {
            throw new Exception("PostgreSQL backup failed. Error: " . implode("\n", $output ?? []));
        }

        return $filepath;
    }

    /**
     * Get all backups
     */
    public static function getBackups(): array
    {
        $backupDir = \storage_path('backups');
        if (!is_dir($backupDir)) {
            return [];
        }

        $files = scandir($backupDir);
        $backups = [];

        foreach ($files as $file) {
            if ($file !== '.' && $file !== '..' && str_ends_with($file, '.sql')) {
                $filepath = "{$backupDir}/{$file}";
                $backups[] = [
                    'filename' => $file,
                    'path' => $filepath,
                    'size' => filesize($filepath),
                    'created_at' => filemtime($filepath),
                ];
            }
        }

        return array_reverse($backups);
    }

    /**
     * Delete a backup
     */
    public static function deleteBackup(string $filename): bool
    {
        $filepath = \storage_path("backups/{$filename}");

        if (!file_exists($filepath)) {
            throw new Exception("Backup file not found: {$filename}");
        }

        return unlink($filepath);
    }

    /**
     * Download a backup
     */
    public static function downloadBackup(string $filename): string
    {
        $filepath = \storage_path("backups/{$filename}");

        if (!file_exists($filepath)) {
            throw new Exception("Backup file not found: {$filename}");
        }

        return $filepath;
    }
}
