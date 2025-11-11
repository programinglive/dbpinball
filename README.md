# dbpinball

A powerful, one-click database backup, restore, and copy tool for MySQL and PostgreSQL. Built with Tauri, React, and TypeScript.

## Features

- **Quick Backup**: Choose connection → pick backup type (structure+data, structure only, data only) → produce .sql (optionally zipped)
- **Restore**: Choose target connection → select .sql/.dump → import with options (drop schema first, stop-on-error, etc.)
- **Copy Database**: Source → target → copy all or selected objects (tables, views, functions, triggers)
- **Job History**: List of runs with logs, status, duration, artifacts, re-run button
- **Connection Vault**: Save named connections; credentials encrypted in OS keystore
- **Real-time Progress**: Stream logs and progress updates live to the UI
- **Vendor Tool Support**: Uses native tools (mysqldump, mysql, pg_dump, psql) when available; falls back to internal dumper
- **Cross-platform**: Windows, macOS, Linux (via Tauri)

## Supported Databases

- MySQL 5.7+ / 8.0+
- PostgreSQL 11+

## Getting Started

### Prerequisites

- Node.js 16+
- Rust 1.70+
- MySQL or PostgreSQL (for testing)

### Installation

```bash
# Install dependencies
npm install

# Start development server
npm run tauri dev

# Build for production
npm run tauri build
```

## Architecture

### Frontend (React + TypeScript)

- **Pages**: Connections, Backup, Restore, Copy, Jobs
- **UI Framework**: shadcn/ui with Tailwind CSS
- **State Management**: React hooks + localStorage
- **Real-time Updates**: Tauri event system

### Backend (Rust + Tauri)

- **Database Access**: sqlx with async/await
- **Password Storage**: OS keyring (Keychain/Credential Manager/libsecret)
- **Async Runtime**: Tokio
- **Compression**: ZIP support with async-compression

## Project Structure

```
dbpinball/
├── src/                          # React frontend
│   ├── pages/
│   │   ├── connections.tsx       # Connection management
│   │   ├── backup.tsx            # Backup interface
│   │   ├── restore.tsx           # Restore interface
│   │   ├── copy.tsx              # Database copy
│   │   └── jobs.tsx              # Job history
│   ├── components/ui/            # shadcn/ui components
│   ├── App.tsx                   # Main app component
│   └── index.css                 # Tailwind styles
├── src-tauri/src/                # Rust backend
│   ├── lib.rs                    # Tauri commands
│   ├── models.rs                 # Data models
│   ├── secrets.rs                # Password management
│   └── runners/
│       ├── mod.rs                # Runner dispatcher
│       ├── mysql.rs              # MySQL implementation
│       └── postgres.rs           # PostgreSQL implementation
├── tailwind.config.js            # Tailwind configuration
├── vite.config.ts                # Vite configuration
└── Cargo.toml                    # Rust dependencies
```

## Usage

### Create a Connection

1. Go to **Connections** tab
2. Click **New Connection**
3. Fill in database details (host, port, database, username)
4. Enter password (stored securely in OS keystore)
5. Click **Test** to verify connection

### Backup a Database

1. Go to **Backup** tab
2. Select a connection
3. Choose backup options:
   - Content: Structure, Data, or both
   - Include: Views, Routines, Triggers
   - Compression: ZIP or plain SQL
4. Click **Start Backup**
5. Monitor progress in real-time
6. Backup file saved to `~/dbpinball/backups/`

### Restore a Database

1. Go to **Restore** tab
2. Select target connection
3. Provide path to backup file
4. Choose restore options:
   - Drop schema if exists
   - Stop on first error
5. Click **Start Restore**

### Copy Database

1. Go to **Copy** tab
2. Select source and target connections
3. Choose what to copy (structure, data, or both)
4. Click **Start Copy**

### View Job History

1. Go to **Jobs** tab
2. See all backup, restore, and copy operations
3. Download artifacts or re-run jobs

## Development

### Running Tests

```bash
# Run Rust tests
cargo test -p dbpinball

# Run React tests (when added)
npm test
```

### Building for Production

```bash
npm run tauri build
```

This creates platform-specific installers in `src-tauri/target/release/bundle/`.

## Security

- **Passwords**: Stored in OS keystore (never on disk)
- **Credentials**: Not logged or exposed in UI
- **TLS Support**: Configurable for database connections
- **No Plaintext**: All sensitive data handled securely

## Roadmap

### Phase 1 (Current)
- ✅ MySQL & PostgreSQL backup/restore/copy
- ✅ Connection management with encrypted storage
- ✅ Job history and logging
- ✅ Native tool integration

### Phase 2
- SSH tunnel support
- Cross-engine copy (MySQL ↔ PostgreSQL)
- Type mapping for cross-engine operations
- Parallel table dumps
- Resumable jobs

### Phase 3
- Web UI for remote access
- Database comparison
- Scheduled backups
- Cloud storage integration

## Troubleshooting

### "mysqldump not found"
Ensure MySQL tools are installed and in PATH:
- macOS: `brew install mysql-client`
- Linux: `apt-get install mysql-client` or `yum install mysql`
- Windows: Install MySQL Server or MySQL Tools

### "Connection failed"
- Verify database is running and accessible
- Check hostname, port, username, password
- Ensure firewall allows connection
- For remote databases, verify network connectivity

### "Permission denied" on backup file
- Check write permissions in `~/dbpinball/backups/`
- Ensure sufficient disk space

## Contributing

Contributions welcome! Please:
1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Submit a pull request

## License

MIT

## Support

For issues, questions, or suggestions, please open an issue on GitHub.
