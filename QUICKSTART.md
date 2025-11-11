# dbpinball Quick Start Guide

## 5-Minute Setup

### 1. Install Dependencies

```bash
npm install
```

### 2. Start Development Server

```bash
npm run tauri dev
```

This will:
- Start Vite dev server on `http://localhost:1420`
- Compile Rust backend
- Open Tauri window

### 3. Create Your First Connection

1. Click **Connections** tab
2. Click **New Connection**
3. Fill in:
   - **Name**: "My Database"
   - **Engine**: MySQL or PostgreSQL
   - **Host**: localhost
   - **Port**: 3306 (MySQL) or 5432 (PostgreSQL)
   - **Database**: your_database
   - **Username**: your_username
   - **Password**: your_password
4. Click **Test** to verify
5. Click **Save Connection**

### 4. Backup Your Database

1. Click **Backup** tab
2. Select your connection
3. Choose options (defaults are fine for most cases)
4. Click **Start Backup**
5. Watch progress in real-time
6. Backup saved to `~/dbpinball/backups/`

### 5. Restore a Backup

1. Click **Restore** tab
2. Select target connection
3. Enter path to backup file (e.g., `/Users/you/dbpinball/backups/mysql_mydb_20240101T120000.sql`)
4. Click **Start Restore**

### 6. Copy Database

1. Click **Copy** tab
2. Select source connection
3. Select target connection
4. Click **Start Copy**

## Common Tasks

### Backup Only Structure (No Data)

1. Go to **Backup**
2. Uncheck **Data**
3. Check **Structure**
4. Click **Start Backup**

### Backup with Compression

1. Go to **Backup**
2. Check **Compress (ZIP)**
3. Click **Start Backup**

### Restore with Drop Schema

1. Go to **Restore**
2. Check **Drop schema if exists**
3. Click **Start Restore**

### View Job History

1. Click **Jobs** tab
2. See all operations with timestamps and status
3. Download artifacts or delete jobs

## Troubleshooting

### "Connection failed"

**Problem**: Can't connect to database

**Solutions**:
1. Verify database is running: `mysql -u root -p` or `psql -U postgres`
2. Check credentials are correct
3. Verify host/port are correct
4. Check firewall isn't blocking connection

### "mysqldump not found"

**Problem**: Backup fails with "mysqldump not found"

**Solutions**:
- **macOS**: `brew install mysql-client`
- **Linux**: `apt-get install mysql-client` (Ubuntu/Debian) or `yum install mysql` (RHEL/CentOS)
- **Windows**: Install MySQL Server or MySQL Tools

### "psql not found"

**Problem**: Restore fails with "psql not found"

**Solutions**:
- **macOS**: `brew install postgresql`
- **Linux**: `apt-get install postgresql-client` (Ubuntu/Debian)
- **Windows**: Install PostgreSQL

### "Permission denied" on backup file

**Problem**: Can't write backup file

**Solutions**:
1. Check `~/dbpinball/backups/` directory exists and is writable
2. Create directory: `mkdir -p ~/dbpinball/backups`
3. Check disk space: `df -h`

### "Stop on first error" during restore

**Problem**: Restore stops on first SQL error

**Solutions**:
1. Uncheck **Stop on first error** to continue despite errors
2. Check backup file is valid SQL
3. Verify target database schema is compatible

## Tips & Tricks

### Backup Before Major Changes

Always backup before:
- Schema migrations
- Data imports
- Major updates
- Production deployments

### Use Meaningful Connection Names

Instead of "db1", use:
- "Production MySQL"
- "Staging PostgreSQL"
- "Local Dev"

### Monitor Large Backups

For large databases:
1. Check disk space before starting
2. Watch progress logs in real-time
3. Don't close app until complete

### Automate with Scripts

Backup file path is printed to logs. Use in scripts:

```bash
# Get latest backup
LATEST=$(ls -t ~/dbpinball/backups/mysql_mydb*.sql | head -1)
echo "Latest backup: $LATEST"
```

## Next Steps

1. **Read the README**: Full feature documentation
2. **Check Architecture**: Understand how it works
3. **Explore Code**: Look at React components and Rust modules
4. **Build for Production**: `npm run tauri build`

## Getting Help

- Check **Troubleshooting** section above
- Read full README.md
- Review ARCHITECTURE.md for technical details
- Check Tauri docs: https://tauri.app/
- Check React docs: https://react.dev/

## Building for Production

```bash
npm run tauri build
```

Creates installers in `src-tauri/target/release/bundle/`:
- Windows: `.msi` installer
- macOS: `.dmg` bundle
- Linux: `.AppImage` or `.deb`

## Development Commands

```bash
# Start dev server
npm run tauri dev

# Build for production
npm run tauri build

# Check Rust code
cargo check

# Run Rust tests
cargo test

# Format code
cargo fmt
npm run format

# Lint code
cargo clippy
npm run lint
```

Happy backing up! 🎉
