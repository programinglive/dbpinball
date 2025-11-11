# dbpinball

A streamlined database workspace inspired by TablePlus, built with Tauri, React, and TypeScript. Manage connections, inspect available databases, and prepare for backup/restore workflows.

## Features

- **TablePlus-style Workspace**: Double-click or connect from the connections list to enter a full-screen workspace overlay for the selected database server.
- **Database Sidebar**: View available databases fetched via the new `list_databases_cmd` backend command.
- **Connection Inspector**: Review connection metadata (host, port, user, SSL status) alongside the workspace canvas.
- **Connection Vault**: Save named connections; credentials encrypted in the OS keystore.
- **Future Operations (Roadmap)**: Backup, restore, copy, and job history flows remain planned and are tracked in the roadmap below.
- **Cross-platform**: Windows, macOS, Linux (via Tauri).

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

- **Pages**: Connections page with TablePlus-inspired workspace overlay
- **UI Framework**: shadcn/ui with Tailwind CSS v4
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
│   │   ├── connections.tsx       # Connection management + workspace overlay
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

### Manage Connections & Open Workspace

1. Open the **Connections** page (default view).
2. Use the **+** toolbar button to add a MySQL or PostgreSQL connection.
3. Fill in database details (host, port, database, username, password).
4. Save and optionally test the connection.
5. Double-click a connection (or choose **Connect** from the context menu) to open the TablePlus-style workspace overlay.
6. Browse databases from the left sidebar; connection details remain visible in the inspector on the right.

### Planned Workflows

Backup, restore, copy, and job history features are under active development and will return as part of upcoming releases.

## Development

### Running Tests

```bash
# Run backend (Rust) tests
cargo test -p dbpinball

# Run frontend (Vitest) tests
npm run test
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
- ✅ Connection management with encrypted storage
- ✅ TablePlus-inspired workspace overlay and database listing
- ✅ Vitest-based UI regression tests
- 🚧 Backup/restore/copy flows (in progress)

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
