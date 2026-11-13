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

    public function test_mysql_rejects_lf_in_password(): void
    {
        $this->expectException(Exception::class);
        $this->expectExceptionMessage('password contains a line break');

        ExposedBackupManager::exposeBuildMysqlCredentials([
            'username' => 'root',
            'password' => "line1\nline2",
            'host' => 'localhost',
            'port' => 3306,
        ]);
    }

    public function test_mysql_rejects_cr_in_password(): void
    {
        $this->expectException(Exception::class);
        $this->expectExceptionMessage('password contains a line break');

        ExposedBackupManager::exposeBuildMysqlCredentials([
            'username' => 'root',
            'password' => "line1\rline2",
            'host' => 'localhost',
            'port' => 3306,
        ]);
    }

    public function test_mysql_rejects_injected_option_directive(): void
    {
        $this->expectException(Exception::class);
        $this->expectExceptionMessage('contains a line break');

        ExposedBackupManager::exposeBuildMysqlCredentials([
            'username' => 'root',
            'password' => "normal\"\nquick=false\nsafe-updates=FALSE\n#",
            'host' => 'localhost',
            'port' => 3306,
        ]);
    }

    public function test_pgpass_rejects_lf_in_password(): void
    {
        $this->expectException(Exception::class);
        $this->expectExceptionMessage('password contains a line break');

        ExposedBackupManager::exposeBuildPgPassEntry([
            'host' => 'localhost',
            'port' => 5432,
            'database' => 'app',
            'username' => 'postgres',
            'password' => "line1\nline2",
        ]);
    }

    public function test_pgpass_rejects_cr_in_field(): void
    {
        $this->expectException(Exception::class);
        $this->expectExceptionMessage('host contains a line break');

        ExposedBackupManager::exposeBuildPgPassEntry([
            'host' => "local\rhost",
            'port' => 5432,
            'database' => 'app',
            'username' => 'postgres',
            'password' => 'secret',
        ]);
    }

    public function test_pgpass_rejects_wildcard_entry_injection(): void
    {
        $this->expectException(Exception::class);
        $this->expectExceptionMessage('contains a line break');

        ExposedBackupManager::exposeBuildPgPassEntry([
            'host' => 'localhost',
            'port' => 5432,
            'database' => 'app',
            'username' => 'postgres',
            'password' => "x\n*:*:*:*:evil",
        ]);
    }

    public function test_validation_error_does_not_leak_secret(): void
    {
        try {
            ExposedBackupManager::exposeBuildMysqlCredentials([
                'username' => 'root',
                'password' => "super-secret\nvalue",
                'host' => 'localhost',
                'port' => 3306,
            ]);

            $this->fail('Expected the credential builder to reject the line break.');
        } catch (Exception $exception) {
            $this->assertStringNotContainsString('super-secret', $exception->getMessage());
            $this->assertStringNotContainsString('value', $exception->getMessage());
            $this->assertStringContainsString('password', $exception->getMessage());
        }
    }

    public function test_download_backup_returns_valid_file(): void
    {
        $file = $this->backupDirectory.'/valid.sql';
        file_put_contents($file, '-- valid');

        $this->assertSame($file, BackupManager::downloadBackup('valid.sql'));
    }

    public function test_download_backup_rejects_external_symlink(): void
    {
        if (! $this->canCreateSymlinks()) {
            $this->markTestSkipped('Symlink creation is not available in this environment.');
        }

        $outside = $this->backupDirectory.DIRECTORY_SEPARATOR.'outside.txt';
        file_put_contents($outside, 'outside');

        $link = $this->backupDirectory.DIRECTORY_SEPARATOR.'external.sql';
        symlink($outside, $link);

        $this->expectException(Exception::class);
        $this->expectExceptionMessage('Backup file must not be a symbolic link.');

        BackupManager::downloadBackup('external.sql');
    }

    public function test_download_backup_rejects_internal_symlink(): void
    {
        if (! $this->canCreateSymlinks()) {
            $this->markTestSkipped('Symlink creation is not available in this environment.');
        }

        $target = $this->backupDirectory.DIRECTORY_SEPARATOR.'actual.sql';
        file_put_contents($target, '-- actual');

        $link = $this->backupDirectory.DIRECTORY_SEPARATOR.'internal.sql';
        symlink($target, $link);

        $this->expectException(Exception::class);
        $this->expectExceptionMessage('Backup file must not be a symbolic link.');

        BackupManager::downloadBackup('internal.sql');
    }

    public function test_temporary_credentials_removed_after_failure(): void
    {
        config(['database.connections.failing_mysql' => [
            'driver' => 'mysql',
            'host' => 'localhost',
            'port' => 3306,
            'database' => 'app',
            'username' => 'root',
            'password' => 'secret',
        ]]);

        FailingBackupManager::$captured = [];

        try {
            FailingBackupManager::backup('failing_mysql');
            $this->fail('Expected the simulated dump failure.');
        } catch (Exception $exception) {
            $this->assertSame('simulated failure', $exception->getMessage());
        }

        $this->assertArrayHasKey('credentials_path', FailingBackupManager::$captured);
        $this->assertFileDoesNotExist(FailingBackupManager::$captured['credentials_path']);
    }

    private function canCreateSymlinks(): bool
    {
        $target = $this->backupDirectory.DIRECTORY_SEPARATOR.'symlink_probe_target';
        $link = $this->backupDirectory.DIRECTORY_SEPARATOR.'symlink_probe_link';

        file_put_contents($target, 'probe');

        $created = @symlink($target, $link);

        if ($created) {
            @unlink($link);
        }

        @unlink($target);

        return $created;
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

class FailingBackupManager extends BackupManager
{
    /** @var array<string, mixed> */
    public static array $captured = [];

    protected static function findExecutable(string $name): string
    {
        return PHP_BINARY;
    }

    protected static function runDump(
        string $binary,
        array $arguments,
        array $environment,
        string $destination,
        array $secrets = []
    ): void {
        foreach ($arguments as $argument) {
            if (str_starts_with($argument, '--defaults-extra-file=')) {
                self::$captured['credentials_path'] = substr($argument, strlen('--defaults-extra-file='));
            }
        }

        if (isset($environment['PGPASSFILE'])) {
            self::$captured['pgpass_path'] = $environment['PGPASSFILE'];
        }

        throw new Exception('simulated failure');
    }
}
