<?php

namespace ProgrammingLive\LaravelSimpleBackup;

use Exception;
use Symfony\Component\Process\Exception\ProcessTimedOutException;
use Symfony\Component\Process\ExecutableFinder;
use Symfony\Component\Process\Process;
use Throwable;

class BackupManager
{
    private const DEFAULT_TIMEOUT_SECONDS = 3600;

    private const MAX_ERROR_OUTPUT_BYTES = 4096;

    /**
     * Backup the database
     */
    public static function backup(?string $connection = null): string
    {
        $connection = $connection ?? \config('database.default');
        $config = \config("database.connections.{$connection}");

        if (! $config) {
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
     * Backup MySQL database.
     *
     * Credentials are written to a private option file and passed with
     * --defaults-extra-file. The password never appears in the argument list,
     * the environment, or the process output.
     */
    private static function backupMysql(array $config): string
    {
        $binary = static::findExecutable('mysqldump');
        $password = (string) ($config['password'] ?? '');
        $database = (string) ($config['database'] ?? '');

        self::assertDatabaseArgument($database);

        $credentialsFile = static::createPrivateFile(static::buildMysqlCredentials($config));

        try {
            $arguments = static::buildMysqlArguments($config, $credentialsFile);
            $destination = static::buildBackupFilePath($database);

            static::runDump($binary, $arguments, [], $destination, [$password]);

            return $destination;
        } finally {
            static::removePrivateFile($credentialsFile);
        }
    }

    /**
     * Backup PostgreSQL database.
     *
     * The password is written to a private .pgpass file and passed with the
     * PGPASSFILE environment variable. It never appears in the argument list or
     * the process output.
     */
    private static function backupPostgres(array $config): string
    {
        $binary = static::findExecutable('pg_dump');
        $password = (string) ($config['password'] ?? '');
        $database = (string) ($config['database'] ?? '');

        $pgPassFile = static::createPrivateFile(static::buildPgPassEntry($config));

        try {
            $arguments = static::buildPostgresArguments($config);
            $environment = ['PGPASSFILE' => $pgPassFile];
            $destination = static::buildBackupFilePath($database);

            static::runDump($binary, $arguments, $environment, $destination, [$password]);

            return $destination;
        } finally {
            static::removePrivateFile($pgPassFile);
        }
    }

    /**
     * Build the MySQL option file contents.
     *
     * CR and LF are rejected before writing: a line break inside a quoted value
     * would end the value and let the remainder be parsed as a new directive.
     */
    protected static function buildMysqlCredentials(array $config): string
    {
        $username = (string) ($config['username'] ?? 'root');
        $password = (string) ($config['password'] ?? '');
        $host = (string) ($config['host'] ?? 'localhost');

        static::assertSingleLine('username', $username);
        static::assertSingleLine('password', $password);
        static::assertSingleLine('host', $host);

        $lines = [
            '[client]',
            'user="'.static::escapeOptionFileValue($username).'"',
            'password="'.static::escapeOptionFileValue($password).'"',
            'host="'.static::escapeOptionFileValue($host).'"',
            'port='.(int) ($config['port'] ?? 3306),
        ];

        return implode("\n", $lines)."\n";
    }

    /**
     * Build the mysqldump argument array.
     *
     * The array form is executed without a shell, so every value stays a single
     * argument no matter which characters it contains.
     *
     * @return array<int, string>
     */
    protected static function buildMysqlArguments(array $config, string $credentialsFile): array
    {
        return [
            '--defaults-extra-file='.$credentialsFile,
            '--host='.(string) ($config['host'] ?? 'localhost'),
            '--port='.(string) ($config['port'] ?? '3306'),
            '--user='.(string) ($config['username'] ?? 'root'),
            (string) ($config['database'] ?? ''),
        ];
    }

    /**
     * Build the .pgpass entry.
     *
     * CR and LF are rejected in every field: the file is line based, so a line
     * break would terminate the record and let the remainder become a second
     * entry.
     */
    protected static function buildPgPassEntry(array $config): string
    {
        $fields = [
            'host' => (string) ($config['host'] ?? 'localhost'),
            'port' => (string) ($config['port'] ?? 5432),
            'database' => (string) ($config['database'] ?? ''),
            'username' => (string) ($config['username'] ?? 'postgres'),
            'password' => (string) ($config['password'] ?? ''),
        ];

        foreach ($fields as $field => $value) {
            static::assertSingleLine($field, $value);
        }

        $escaped = array_map(static function (string $value): string {
            return str_replace(['\\', ':'], ['\\\\', '\\:'], $value);
        }, array_values($fields));

        return implode(':', $escaped)."\n";
    }

    /**
     * Build the pg_dump argument array.
     *
     * @return array<int, string>
     */
    protected static function buildPostgresArguments(array $config): array
    {
        return [
            '--host='.(string) ($config['host'] ?? 'localhost'),
            '--port='.(string) ($config['port'] ?? '5432'),
            '--username='.(string) ($config['username'] ?? 'postgres'),
            '--dbname='.(string) ($config['database'] ?? ''),
        ];
    }

    /**
     * Execute a dump process and stream stdout into the destination file.
     *
     * Output is written to a temporary file next to the destination and renamed
     * into place only after the process succeeds, so a failed run never leaves a
     * partial backup behind.
     *
     * @param  array<int, string>  $arguments
     * @param  array<string, string>  $environment
     * @param  array<int, string>  $secrets
     */
    protected static function runDump(
        string $binary,
        array $arguments,
        array $environment,
        string $destination,
        array $secrets = []
    ): void {
        $temporary = $destination.'.part.'.bin2hex(random_bytes(8));

        $handle = @fopen($temporary, 'xb');

        if ($handle === false) {
            throw new Exception('Unable to open the backup file for writing.');
        }

        $process = new Process(array_merge([$binary], $arguments));
        $process->setTimeout(static::resolveTimeout());

        if ($environment !== []) {
            $process->setEnv($environment);
        }

        $errorOutput = '';
        $writeFailed = false;

        try {
            $process->run(static function (string $type, string $buffer) use ($handle, &$errorOutput, &$writeFailed): void {
                if ($type === Process::OUT) {
                    if (fwrite($handle, $buffer) === false) {
                        $writeFailed = true;
                    }

                    return;
                }

                $errorOutput .= $buffer;

                if (strlen($errorOutput) > self::MAX_ERROR_OUTPUT_BYTES) {
                    $errorOutput = substr($errorOutput, -self::MAX_ERROR_OUTPUT_BYTES);
                }
            });
        } catch (ProcessTimedOutException $exception) {
            fclose($handle);
            @unlink($temporary);

            throw new Exception('The database dump timed out before completion.');
        } catch (Throwable $exception) {
            fclose($handle);
            @unlink($temporary);

            throw new Exception('The database dump could not be started.');
        }

        fclose($handle);

        if ($writeFailed) {
            @unlink($temporary);

            throw new Exception('Unable to write the backup file.');
        }

        if (! $process->isSuccessful()) {
            @unlink($temporary);

            $exitCode = $process->getExitCode() ?? -1;
            $message = static::redact(trim($errorOutput), $secrets);
            $suffix = $message !== '' ? ': '.$message : '';

            throw new Exception("Database dump failed with exit code {$exitCode}{$suffix}");
        }

        if (! @rename($temporary, $destination)) {
            @unlink($temporary);

            throw new Exception('Unable to finalize the backup file.');
        }
    }

    /**
     * Resolve an executable from PATH.
     *
     * The executable name is fixed by the caller, never taken from configuration.
     */
    protected static function findExecutable(string $name): string
    {
        $finder = new ExecutableFinder();
        $path = $finder->find($name);

        if (! is_string($path) || $path === '') {
            throw new Exception("Required executable '{$name}' was not found in PATH.");
        }

        return $path;
    }

    /**
     * Create a private temporary file and return its path.
     */
    protected static function createPrivateFile(string $contents): string
    {
        $base = rtrim(sys_get_temp_dir(), DIRECTORY_SEPARATOR);
        $directory = $base.DIRECTORY_SEPARATOR.'dbpinball_'.bin2hex(random_bytes(16));

        if (! @mkdir($directory, 0700, true) && ! is_dir($directory)) {
            throw new Exception('Unable to create a temporary directory for credentials.');
        }

        $path = $directory.DIRECTORY_SEPARATOR.'credentials';

        $handle = @fopen($path, 'xb');

        if ($handle === false) {
            @rmdir($directory);

            throw new Exception('Unable to create a temporary credentials file.');
        }

        try {
            @chmod($path, 0600);

            if (@fwrite($handle, $contents) === false) {
                throw new Exception('Unable to write the temporary credentials file.');
            }
        } catch (Throwable $exception) {
            fclose($handle);
            @unlink($path);
            @rmdir($directory);

            throw $exception;
        }

        fclose($handle);

        return $path;
    }

    /**
     * Remove a private temporary file and its directory.
     */
    protected static function removePrivateFile(string $path): void
    {
        @unlink($path);
        @rmdir(dirname($path));
    }

    /**
     * Resolve the backup directory, honoring configuration.
     */
    protected static function resolveBackupDirectory(): string
    {
        $configured = \config('backup.backup_path');
        $directory = is_string($configured) && $configured !== ''
            ? $configured
            : \storage_path('backups');

        if (! is_dir($directory)) {
            if (! @mkdir($directory, 0700, true) && ! is_dir($directory)) {
                throw new Exception('Unable to create the backup directory.');
            }
        }

        if (! is_dir($directory)) {
            throw new Exception('The backup directory is not accessible.');
        }

        return rtrim($directory, '/\\');
    }

    /**
     * Build the destination path for a new backup.
     */
    protected static function buildBackupFilePath(string $database): string
    {
        $directory = static::resolveBackupDirectory();
        $component = static::sanitizeFilenameComponent($database);
        $filename = 'backup_'.$component.'_'.date('Y-m-d_H-i-s').'.sql';

        return $directory.'/'.$filename;
    }

    /**
     * Reduce an untrusted value to a safe filename component.
     *
     * Dots are removed entirely so a value can never become "." or "..".
     */
    protected static function sanitizeFilenameComponent(string $value): string
    {
        $sanitized = (string) preg_replace('/[^A-Za-z0-9_-]/', '_', $value);
        $sanitized = trim($sanitized, '_');

        return $sanitized === '' ? 'database' : $sanitized;
    }

    /**
     * Resolve an existing backup file inside the backup directory.
     *
     * Rejects path traversal, symbolic links, non-regular files, and paths that
     * resolve outside the backup directory.
     *
     * Residual risk: these checks are not atomic with the caller's subsequent
     * open. An attacker who can write to the backup directory could still
     * replace the file between this call and the read (a time-of-check /
     * time-of-use race). Closing that window would require returning an open
     * file descriptor, which would change the public API. The backup directory
     * is therefore treated as a trusted, non-shared location.
     */
    protected static function resolveBackupFile(string $filename): string
    {
        $isBareName = $filename !== ''
            && $filename !== '.'
            && $filename !== '..'
            && basename($filename) === $filename
            && ! str_contains($filename, '/')
            && ! str_contains($filename, '\\');

        if (! $isBareName) {
            throw new Exception('Invalid backup filename.');
        }

        $directory = static::resolveBackupDirectory();
        $filepath = $directory.'/'.$filename;

        // lstat does not follow symlinks, so a link is detected before is_file.
        if (@lstat($filepath) === false) {
            throw new Exception("Backup file not found: {$filename}");
        }

        if (is_link($filepath)) {
            throw new Exception('Backup file must not be a symbolic link.');
        }

        if (! is_file($filepath)) {
            throw new Exception('Backup path is not a regular file.');
        }

        $realFile = realpath($filepath);
        $realDirectory = realpath($directory);

        if ($realFile === false || $realDirectory === false) {
            throw new Exception('Backup file could not be resolved.');
        }

        if (! static::isPathInside($realFile, $realDirectory)) {
            throw new Exception('Backup file is outside the backup directory.');
        }

        return $filepath;
    }

    /**
     * Determine whether a resolved path is inside a resolved directory.
     */
    protected static function isPathInside(string $path, string $directory): bool
    {
        $prefix = rtrim($directory, '/\\').DIRECTORY_SEPARATOR;

        if (DIRECTORY_SEPARATOR === '\\') {
            return str_starts_with(strtolower($path), strtolower($prefix));
        }

        return str_starts_with($path, $prefix);
    }

    /**
     * Reject line breaks in a value written to a credential file.
     *
     * The message names only the field, never the value, so a rejected secret
     * cannot leak through an exception or a log.
     */
    protected static function assertSingleLine(string $field, string $value): void
    {
        if (str_contains($value, "\n") || str_contains($value, "\r")) {
            throw new Exception("The database {$field} contains a line break and cannot be written to a credential file.");
        }
    }

    /**
     * Escape a value for a MySQL option file.
     */
    protected static function escapeOptionFileValue(string $value): string
    {
        return str_replace(['\\', '"'], ['\\\\', '\\"'], $value);
    }

    /**
     * Remove secret values from a diagnostic message.
     *
     * @param  array<int, string>  $secrets
     */
    protected static function redact(string $text, array $secrets): string
    {
        foreach ($secrets as $secret) {
            if (is_string($secret) && $secret !== '') {
                $text = str_replace($secret, '***', $text);
            }
        }

        return $text;
    }

    /**
     * Resolve the process timeout in seconds.
     */
    protected static function resolveTimeout(): float
    {
        $configured = \config('backup.timeout');

        if (is_numeric($configured) && (float) $configured > 0) {
            return (float) $configured;
        }

        return (float) self::DEFAULT_TIMEOUT_SECONDS;
    }

    /**
     * Reject a database name that could be parsed as an option.
     */
    protected static function assertDatabaseArgument(string $database): void
    {
        if ($database === '' || str_starts_with($database, '-')) {
            throw new Exception('Invalid database name for backup.');
        }
    }

    /**
     * Get all backups
     *
     * @return array<int, array{filename: string, path: string, size: int|false, created_at: int|false}>
     */
    public static function getBackups(): array
    {
        $backupDir = static::resolveBackupDirectory();

        if (! is_dir($backupDir)) {
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
        $filepath = static::resolveBackupFile($filename);

        return unlink($filepath);
    }

    /**
     * Download a backup
     */
    public static function downloadBackup(string $filename): string
    {
        return static::resolveBackupFile($filename);
    }
}
