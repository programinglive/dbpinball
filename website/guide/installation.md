# Installation

DBPinball is a desktop application with an optional command-line interface. You
do not need Rust, Cargo, Node.js or a source checkout to install it.

::: warning Public release status
The installers are built and verified by CI, but no public release has been
published yet. Until the first authorized release, the download links below
point to the [releases page](https://github.com/programinglive/dbpinball/releases),
which is empty. Build from source only if you are developing DBPinball; see
[Building from source](/development/building).
:::

## 1. Download DBPinball

Open the [releases page](https://github.com/programinglive/dbpinball/releases)
and pick the file for your platform:

| Platform | Desktop | CLI |
| --- | --- | --- |
| Windows 10/11 (x64) | `dbpinball_<version>_x64-setup.exe` | `dbpinball-cli-<version>-x64.zip` |
| Linux (x64) | `dbpinball_<version>_amd64.deb` or `.AppImage` | `dbpinball-cli-<version>-x64-linux.tar.gz` |
| macOS | Not packaged yet; build from source | Not packaged yet |

Every release also publishes `SHA256SUMS.txt`. Verify a download before running
it:

```powershell
# Windows
Get-FileHash .\dbpinball_0.1.6_x64-setup.exe -Algorithm SHA256
```

```bash
# Linux
sha256sum -c SHA256SUMS.txt
```

## 2. Install the desktop application

### Windows

1. Run `dbpinball_<version>_x64-setup.exe`.
2. The installer installs for the current user only, so it does not ask for
   administrator rights.
3. It creates a Start Menu entry and a normal uninstaller in **Settings → Apps**.

Windows SmartScreen may warn that the publisher is unknown, because the
installer is not code-signed yet. Choose **More info → Run anyway** only if the
checksum matches the published value.

### Linux

```bash
# Debian or Ubuntu
sudo apt install ./dbpinball_<version>_amd64.deb

# Or run the AppImage without installing
chmod +x dbpinball_<version>_amd64.AppImage
./dbpinball_<version>_amd64.AppImage
```

## 3. Install the CLI (optional)

The CLI is a separate download. It is not installed by the desktop installer.

### Windows

1. Extract `dbpinball-cli-<version>-x64.zip`.
2. Open PowerShell in the extracted folder.
3. Run:

```powershell
powershell -ExecutionPolicy Bypass -File .\install-cli.ps1
```

The script copies `dbpinball.exe` to `%LOCALAPPDATA%\dbpinball\bin` and adds that
folder to your **user** PATH. It needs no administrator rights, never touches
machine-wide settings, and leaves your other PATH entries unchanged.

Open a **new** terminal afterwards. The current one keeps its old environment.

### Linux

```bash
tar -xzf dbpinball-cli-<version>-x64-linux.tar.gz
sudo install -m 0755 dbpinball /usr/local/bin/dbpinball
```

## 4. Launch the application

Start **DBPinball** from the Start Menu (Windows) or your application launcher
(Linux). The first window is the connection list.

## 5. Verify the installation

```powershell
dbpinball --version
dbpinball doctor
dbpinball --help
```

`dbpinball doctor` reports the version, the configuration directory, the
credential store and whether the optional `mysqldump`/`pg_dump` tools are on
`PATH`. It never prints a password.

If the desktop application starts, the installation is complete. Continue with
the [quick start](/guide/quick-start).

## 6. Upgrade

- **Windows desktop:** run the newer installer. It replaces the previous
  version and keeps your connection profiles.
- **Windows CLI:** extract the new archive and run `install-cli.ps1` again. It
  replaces the binary and leaves the PATH entry untouched.
- **Linux:** install the newer package, or replace the AppImage.

## 7. Uninstall

- **Windows desktop:** **Settings → Apps → DBPinball → Uninstall**.
- **Windows CLI:**

```powershell
powershell -ExecutionPolicy Bypass -File .\install-cli.ps1 -Uninstall
```

This removes the binary and the PATH entry it added. Other PATH entries are
preserved.

- **Linux:** `sudo apt remove dbpinball`, or delete the AppImage.

Connection profiles live in the configuration directory and are not removed by
the uninstaller. See [Where the data lives](#where-the-data-lives).

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `'dbpinball' is not recognized` | The CLI is not installed, or the terminal was opened before installing it | Run `install-cli.ps1`, then open a new terminal |
| `dbpinball` runs an old version | An older copy is earlier on `PATH` | Run `where dbpinball` and remove the stale copy |
| SmartScreen blocks the installer | The installer is not code-signed yet | Verify the checksum, then **More info → Run anyway** |
| Antivirus quarantines `dbpinball.exe` | Heuristic false positive on an unsigned binary | Verify the checksum and report it; the release binary carries full version metadata |
| The desktop window is blank | The frontend bundle did not load | Reinstall, then report the version from `dbpinball --version` |

More in [Troubleshooting](/guide/troubleshooting).

## Where the data lives

| Purpose | Location |
| --- | --- |
| Connection profiles | `dirs::config_dir()/dbpinball/connections.json` |
| Passwords | The OS keyring, service `dbpinball`, account = profile id |

On Windows the configuration directory is `%APPDATA%\dbpinball`; on macOS it is
`~/Library/Application Support/dbpinball`; on Linux it is `~/.config/dbpinball`.

The CLI accepts `--config-dir <PATH>` to override the location for tests and
advanced use. The desktop always uses the default, so the two cannot drift.
