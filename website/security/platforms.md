# Platforms

## Verification status

| Platform | Desktop | CLI | Verified by |
| --- | --- | --- | --- |
| Windows | Supported | Supported | Installer and CLI archive built and installed on Windows 10/11 x64; `dbpinball --version`, `doctor` and `--help` verified from a fresh environment |
| Linux | Supported | Supported | CI (`ubuntu-latest`) builds the CLI archive and the `.deb`/AppImage packages |
| macOS | Supported by the code | Supported by the code | **Not exercised in CI, not packaged** |

The lock uses `ps -p` on Unix and `tasklist` on Windows. The keyring uses the
platform backend through the `keyring` crate.

## What "not exercised" means

macOS is not claimed as verified. The code paths are the same Unix paths that CI
runs on Linux, but a platform that has not been run is not a platform that has
been proven.

## Configuration paths

| Platform | Config directory |
| --- | --- |
| Windows | `%APPDATA%\dbpinball` |
| macOS | `~/Library/Application Support/dbpinball` |
| Linux | `~/.config/dbpinball` |

## Related

- [Limitations](/security/limitations)
- [Requirements](/guide/requirements)
