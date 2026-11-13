<?php

namespace ProgrammingLive\LaravelSimpleBackup\Tests;

use Exception;
use ProgrammingLive\LaravelSimpleBackup\BackupManager;

class SecurityTest extends TestCase
{
    private string $backupDirectory;

    protected function setUp(): void
    {
        parent::setUp();

        $this->backupDirectory = sys_get_temp_dir().DIRECTORY_SEPARATOR.'dbpinball_test_'.bin2hex(random_bytes(8));
        mkdir($this->backupDirectory, 0700, true);

        config(['backup.backup_path' => $this->backupDirectory]);
        config(['backup.timeout' => 5]);

        CapturingBackupManager::$captured = [];
        CapturingBackupManager::$binary = PHP_BINARY;
    }

    protected function tearDown(): void
    {
        $this->removeDirectory($this->backupDirectory);

        parent::tearDown();
    }

    private function removeDirectory(string $directory): void
    {
        if (! is_dir($directory)) {
            return;
        }

        foreach (glob($directory.DIRECTORY_SEPARATOR.'*') ?: [] as $file) {
            if (is_file($file)) {
                unlink($file);
            }
        }

        @rmdir($directory);
    }

    public function test_mysql_arguments_treat_metacharacters_as_data(): void
    {
        $config = [
            'driver' => 'mysql',
            'host' => 'localhost; touch /tmp/pwned',
            'port' => 3306,
            'database' => 'db; rm -rf /',
            'username' => 'user | cat /etc/passwd',
            'password' => 'p@ss; touch /tmp/pwned',
        ];

        $arguments = ExposedBackupManager::exposeBuildMysqlArguments($config, '/tmp/credentials');

        $this->assertContains('--host=localhost; touch /tmp/pwned', $arguments);
        $this->assertContains('--user=user | cat /etc/passwd', $arguments);
        $this->assertContains('db; rm -rf /', $arguments);
        $this->assertNotContains('p@ss; touch /tmp/pwned', $arguments);

        foreach ($arguments as $argument) {
            $this->assertIsString($argument);
        }
    }

    public function test_mysql_credentials_escape_quotes_and_backslashes(): void
    {
        $config = [
            'username' => 'user"name',
            'password' => 'p"a\\ss',
            'host' => 'localhost',
            'port' => 3306,
        ];

        $contents = ExposedBackupManager::exposeBuildMysqlCredentials($config);

        $this->assertStringContainsString('user="user\"name"', $contents);
        $this->assertStringContainsString('password="p\"a\\\\ss"', $contents);
    }

    public function test_pgpass_entry_escapes_colons_and_backslashes(): void
    {
        $config = [
            'host' => 'localhost',
            'port' => 5432,
            'database' => 'app',
            'username' => 'postgres',
            'password' => 'pa:ss\\word',
        ];

        $entry = ExposedBackupManager::exposeBuildPgPassEntry($config);

        $this->assertStringContainsString('pa\\:ss\\\\word', $entry);
    }

    public function test_sanitize_filename_component_removes_path_separators(): void
    {
        $sanitized = ExposedBackupManager::exposeSanitizeFilenameComponent('../../etc/passwd');

        $this->assertStringNotContainsString('/', $sanitized);
        $this->assertStringNotContainsString('\\', $sanitized);
        $this->assertStringNotContainsString('..', $sanitized);
        $this->assertSame('etc_passwd', $sanitized);
    }

    public function test_redact_removes_secret_values(): void
    {
        $this->assertSame('error: ***', ExposedBackupManager::exposeRedact('error: secret123', ['secret123']));
    }

    public function test_run_dump_writes_output_atomically(): void
    {
        $destination = $this->backupDirectory.DIRECTORY_SEPARATOR.'atomic.sql';

        ExposedBackupManager::exposeRunDump(PHP_BINARY, ['-r', 'echo "hello";'], [], $destination);

        $this->assertSame('hello', file_get_contents($destination));
        $this->assertSame([], glob($destination.'.part.*') ?: []);
    }

    public function test_run_dump_removes_partial_file_on_failure(): void
    {
        $destination = $this->backupDirectory.DIRECTORY_SEPARATOR.'failed.sql';

        try {
            ExposedBackupManager::exposeRunDump(
                PHP_BINARY,
                ['-r', 'fwrite(STDOUT, "partial"); fwrite(STDERR, "boom"); exit(7);'],
                [],
                $destination
            );

            $this->fail('Expected the dump to fail.');
        } catch (Exception $exception) {
            $this->assertStringContainsString('exit code 7', $exception->getMessage());
            $this->assertStringContainsString('boom', $exception->getMessage());
        }

        $this->assertFileDoesNotExist($destination);
        $this->assertSame([], glob($destination.'.part.*') ?: []);
    }

    public function test_run_dump_redacts_password_in_error(): void
    {
        $destination = $this->backupDirectory.DIRECTORY_SEPARATOR.'redacted.sql';

        try {
            ExposedBackupManager::exposeRunDump(
                PHP_BINARY,
                ['-r', 'fwrite(STDERR, "leaked secret123"); exit(1);'],
                [],
                $destination,
                ['secret123']
            );

            $this->fail('Expected the dump to fail.');
        } catch (Exception $exception) {
            $this->assertStringNotContainsString('secret123', $exception->getMessage());
            $this->assertStringContainsString('***', $exception->getMessage());
        }
    }

    public function test_run_dump_times_out_safely(): void
    {
        config(['backup.timeout' => 0.5]);

        $destination = $this->backupDirectory.DIRECTORY_SEPARATOR.'timeout.sql';

        try {
            ExposedBackupManager::exposeRunDump(PHP_BINARY, ['-r', 'sleep(3);'], [], $destination);

            $this->fail('Expected the dump to time out.');
        } catch (Exception $exception) {
            $this->assertStringContainsString('timed out', $exception->getMessage());
        }

        $this->assertFileDoesNotExist($destination);
        $this->assertSame([], glob($destination.'.part.*') ?: []);
    }

    public function test_run_dump_passes_metacharacters_literally_without_shell(): void
    {
        $destination = $this->backupDirectory.DIRECTORY_SEPARATOR.'literal.sql';
        $marker = $this->backupDirectory.DIRECTORY_SEPARATOR.'pwned';
        $payload = 'a; touch '.$marker;

        ExposedBackupManager::exposeRunDump(
            PHP_BINARY,
            ['-r', 'echo $argv[1];', $payload],
            [],
            $destination
        );

        $this->assertSame($payload, file_get_contents($destination));
        $this->assertFileDoesNotExist($marker);
    }

    public function test_missing_executable_throws_safe_error(): void
    {
        $this->expectException(Exception::class);
        $this->expectExceptionMessage("Required executable 'definitely-not-a-real-binary-xyz' was not found in PATH.");

        ExposedBackupManager::exposeFindExecutable('definitely-not-a-real-binary-xyz');
    }

    public function test_mysql_backup_keeps_password_out_of_arguments_and_cleans_credentials(): void
    {
        config(['database.connections.secure_mysql' => [
            'driver' => 'mysql',
            'host' => 'localhost',
            'port' => 3306,
            'database' => 'app',
            'username' => 'root',
            'password' => 'super-secret-password',
        ]]);

        $path = CapturingBackupManager::backup('secure_mysql');

        $this->assertFileExists($path);
        $this->assertNotContains('super-secret-password', CapturingBackupManager::$captured['arguments']);
        $this->assertStringContainsString('super-secret-password', CapturingBackupManager::$captured['credentials_contents']);
        $this->assertFileDoesNotExist(CapturingBackupManager::$captured['credentials_path']);
    }

    public function test_postgres_backup_uses_pgpassfile_and_cleans_it(): void
    {
        config(['database.connections.secure_pgsql' => [
            'driver' => 'pgsql',
            'host' => 'localhost',
            'port' => 5432,
            'database' => 'app',
            'username' => 'postgres',
            'password' => 'super-secret-password',
        ]]);

        $path = CapturingBackupManager::backup('secure_pgsql');

        $this->assertFileExists($path);
        $this->assertArrayHasKey('PGPASSFILE', CapturingBackupManager::$captured['environment']);
        $this->assertNotContains('super-secret-password', CapturingBackupManager::$captured['arguments']);
        $this->assertStringContainsString('super-secret-password', CapturingBackupManager::$captured['pgpass_contents']);
        $this->assertFileDoesNotExist(CapturingBackupManager::$captured['pgpass_path']);
    }

    public function test_backup_path_with_spaces_works(): void
    {
        $spaced = $this->backupDirectory.DIRECTORY_SEPARATOR.'backup dir with spaces';
        mkdir($spaced, 0700, true);

        config(['backup.backup_path' => $spaced]);

        $destination = $spaced.DIRECTORY_SEPARATOR.'out.sql';

        ExposedBackupManager::exposeRunDump(PHP_BINARY, ['-r', 'echo "spaced";'], [], $destination);

        $this->assertSame('spaced', file_get_contents($destination));
    }

    public function test_delete_backup_rejects_path_traversal(): void
    {
        $this->expectException(Exception::class);
        $this->expectExceptionMessage('Invalid backup filename.');

        BackupManager::deleteBackup('../../config/database.php');
    }

    public function test_download_backup_rejects_path_traversal(): void
    {
        $this->expectException(Exception::class);
        $this->expectExceptionMessage('Invalid backup filename.');

        BackupManager::downloadBackup('../secret.sql');
    }

    public function test_mysql_backup_rejects_option_like_database_name(): void
    {
        config(['database.connections.option_like' => [
            'driver' => 'mysql',
            'host' => 'localhost',
            'port' => 3306,
            'database' => '--help',
            'username' => 'root',
            'password' => '',
        ]]);

        $this->expectException(Exception::class);
        $this->expectExceptionMessage('Invalid database name for backup.');

        CapturingBackupManager::backup('option_like');
    }
}

class CapturingBackupManager extends BackupManager
{
    /** @var array<string, mixed> */
    public static array $captured = [];

    public static string $binary = '';

    protected static function findExecutable(string $name): string
    {
        return self::$binary !== '' ? self::$binary : parent::findExecutable($name);
    }

    protected static function runDump(
        string $binary,
        array $arguments,
        array $environment,
        string $destination,
        array $secrets = []
    ): void {
        self::$captured = [
            'binary' => $binary,
            'arguments' => $arguments,
            'environment' => $environment,
            'destination' => $destination,
            'secrets' => $secrets,
        ];

        foreach ($arguments as $argument) {
            if (str_starts_with($argument, '--defaults-extra-file=')) {
                $path = substr($argument, strlen('--defaults-extra-file='));
                self::$captured['credentials_path'] = $path;
                self::$captured['credentials_contents'] = file_get_contents($path);
            }
        }

        if (isset($environment['PGPASSFILE'])) {
            self::$captured['pgpass_path'] = $environment['PGPASSFILE'];
            self::$captured['pgpass_contents'] = file_get_contents($environment['PGPASSFILE']);
        }

        file_put_contents($destination, '-- fake dump');
    }
}

class ExposedBackupManager extends BackupManager
{
    public static function exposeRunDump(
        string $binary,
        array $arguments,
        array $environment,
        string $destination,
        array $secrets = []
    ): void {
        parent::runDump($binary, $arguments, $environment, $destination, $secrets);
    }

    public static function exposeFindExecutable(string $name): string
    {
        return parent::findExecutable($name);
    }

    public static function exposeBuildMysqlArguments(array $config, string $credentialsFile): array
    {
        return parent::buildMysqlArguments($config, $credentialsFile);
    }

    public static function exposeBuildMysqlCredentials(array $config): string
    {
        return parent::buildMysqlCredentials($config);
    }

    public static function exposeBuildPgPassEntry(array $config): string
    {
        return parent::buildPgPassEntry($config);
    }

    public static function exposeSanitizeFilenameComponent(string $value): string
    {
        return parent::sanitizeFilenameComponent($value);
    }

    public static function exposeRedact(string $text, array $secrets): string
    {
        return parent::redact($text, $secrets);
    }
}
