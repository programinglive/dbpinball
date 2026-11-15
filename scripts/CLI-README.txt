DBPinball CLI
=============

This archive contains the DBPinball command-line interface for Windows x64.

Install (no administrator rights required)
------------------------------------------

1. Extract this archive to any folder.
2. Open PowerShell in that folder.
3. Run:

   powershell -ExecutionPolicy Bypass -File .\install-cli.ps1

The script copies dbpinball.exe to %LOCALAPPDATA%\dbpinball\bin and adds that
folder to your user PATH. Open a new terminal afterwards and run:

   dbpinball --version
   dbpinball doctor
   dbpinball --help

Uninstall
---------

   powershell -ExecutionPolicy Bypass -File .\install-cli.ps1 -Uninstall

This removes the binary and the PATH entry. It never touches other PATH
entries.

Portable use
------------

You can also run dbpinball.exe directly from this folder without installing
anything. The installer only exists so that `dbpinball` resolves from any
terminal.

Where data lives
----------------

Connection profiles: %APPDATA%\dbpinball\connections.json
Passwords: the Windows Credential Manager, service "dbpinball"

Documentation: https://dbpinball.programinglive.com
