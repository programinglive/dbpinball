# dbpinball Development Guide

## Development Environment Setup

### Prerequisites

- Node.js 16+ (check: `node --version`)
- Rust 1.70+ (check: `rustc --version`)
- Git
- A code editor (VS Code recommended)

### Initial Setup

```bash
# Clone or navigate to project
cd d:\code\dbpinball

# Install dependencies
npm install

# Start development server
npm run tauri dev
```

## Project Structure for Developers

### Frontend (React + TypeScript)

**Entry Point:** `src/main.tsx`

```
src/
├── App.tsx                    # Main app with tabs
├── main.tsx                   # React entry point
├── index.css                  # Tailwind styles
├── pages/                     # Page components
│   ├── connections.tsx        # Connection CRUD
│   ├── backup.tsx             # Backup interface
│   ├── restore.tsx            # Restore interface
│   ├── copy.tsx               # Copy interface
│   └── jobs.tsx               # Job history
├── components/ui/             # shadcn/ui components
└── lib/utils.ts               # Utility functions
```

**Key Patterns:**

1. **State Management** - React hooks + localStorage
   ```typescript
   const [connections, setConnections] = useState<DbConnection[]>([]);
   
   useEffect(() => {
     const stored = localStorage.getItem("dbpinball_connections");
     if (stored) setConnections(JSON.parse(stored));
   }, []);
   ```

2. **Tauri Commands** - Invoke Rust functions
   ```typescript
   import { invoke } from "@tauri-apps/api/core";
   
   const result = await invoke("backup", { conn, opts });
   ```

3. **Event Listening** - Real-time progress
   ```typescript
   import { listen } from "@tauri-apps/api/event";
   
   const unlisten = listen<string>("job-progress", (e) => {
     setLogs((prev) => [...prev, e.payload]);
   });
   ```

4. **UI Components** - shadcn/ui
   ```typescript
   import { Button } from "@/components/ui/button";
   import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
   ```

### Backend (Rust + Tauri)

**Entry Point:** `src-tauri/src/lib.rs`

```
src-tauri/src/
├── lib.rs                     # Tauri commands
├── models.rs                  # Data models
├── secrets.rs                 # Password management
└── runners/
    ├── mod.rs                 # Dispatcher
    ├── mysql.rs               # MySQL implementation
    └── postgres.rs            # PostgreSQL implementation
```

**Key Patterns:**

1. **Tauri Commands** - Async functions exposed to frontend
   ```rust
   #[tauri::command]
   async fn backup(
       window: tauri::Window,
       conn: DbConnection,
       opts: BackupOptions,
   ) -> Result<String, String> {
       start_backup(&window, conn, opts)
           .await
           .map_err(|e| e.to_string())
   }
   ```

2. **Progress Events** - Emit to frontend
   ```rust
   fn emit(window: &Window, msg: &str) {
       let _ = window.emit("job-progress", msg);
   }
   ```

3. **Error Handling** - anyhow::Result
   ```rust
   pub async fn backup(...) -> Result<String> {
       let password = secrets::get_password(&conn.id)?;
       let pool = sqlx::MySqlPool::connect(&url).await?;
       Ok(artifact_path)
   }
   ```

4. **Async Operations** - Tokio
   ```rust
   let file = tokio::fs::File::create(path).await?;
   let mut cmd = tokio::process::Command::new(bin);
   tokio::io::copy(&mut reader, &mut writer).await?;
   ```

## Common Development Tasks

### Add a New Page

1. Create `src/pages/mypage.tsx`:
   ```typescript
   export default function MyPage() {
     return <div>My Page</div>;
   }
   ```

2. Import in `src/App.tsx`:
   ```typescript
   import MyPage from "@/pages/mypage";
   ```

3. Add tab:
   ```typescript
   <TabsTrigger value="mypage">My Page</TabsTrigger>
   <TabsContent value="mypage"><MyPage /></TabsContent>
   ```

### Add a New Tauri Command

1. Add function in `src-tauri/src/lib.rs`:
   ```rust
   #[tauri::command]
   async fn my_command(param: String) -> Result<String, String> {
       Ok(format!("Hello, {}", param))
   }
   ```

2. Add to handler:
   ```rust
   .invoke_handler(tauri::generate_handler![
       test_conn,
       backup,
       restore,
       copy_db,
       store_password,
       get_password,
       delete_password,
       my_command,  // Add here
   ])
   ```

3. Call from React:
   ```typescript
   const result = await invoke("my_command", { param: "world" });
   ```

### Add a New UI Component

1. Use shadcn/ui:
   ```bash
   npx shadcn@latest add component-name -y
   ```

2. Import and use:
   ```typescript
   import { MyComponent } from "@/components/ui/my-component";
   ```

### Debug Rust Code

1. Add logging:
   ```rust
   eprintln!("Debug: {:?}", variable);
   ```

2. Run with output:
   ```bash
   npm run tauri dev
   ```

3. Check console for output

### Debug React Code

1. Use browser DevTools:
   - Right-click → Inspect
   - Check Console tab
   - Use debugger

2. Add console logs:
   ```typescript
   console.log("Debug:", variable);
   ```

## Testing

### Manual Testing Checklist

- [ ] Create connection to MySQL
- [ ] Create connection to PostgreSQL
- [ ] Test connection button
- [ ] Backup structure only
- [ ] Backup data only
- [ ] Backup with compression
- [ ] Restore from backup
- [ ] Copy database
- [ ] View job history
- [ ] Delete connection
- [ ] Verify passwords stored securely

### Automated Testing (Future)

```bash
# Rust tests
cargo test -p dbpinball

# React tests (when added)
npm test
```

## Performance Optimization

### Frontend

1. **Memoization** - Prevent unnecessary re-renders
   ```typescript
   const MemoComponent = React.memo(Component);
   ```

2. **useCallback** - Memoize callbacks
   ```typescript
   const handleClick = useCallback(() => {
     // ...
   }, [dependencies]);
   ```

3. **Code Splitting** - Lazy load pages (future)
   ```typescript
   const BackupPage = lazy(() => import("@/pages/backup"));
   ```

### Backend

1. **Connection Pooling** - sqlx manages pools
2. **Streaming** - Don't buffer large files
3. **Async** - Use tokio for I/O
4. **Compression** - Async doesn't block UI

## Debugging Tips

### Rust Errors

1. Check compiler output carefully
2. Use `cargo check` for quick feedback
3. Enable backtrace: `RUST_BACKTRACE=1 cargo test`

### React Errors

1. Check browser console
2. Use React DevTools extension
3. Add console.log statements

### Tauri Communication

1. Check browser DevTools Network tab
2. Add logging in Rust commands
3. Verify event names match

## Code Style

### Rust

- Use `cargo fmt` for formatting
- Use `cargo clippy` for linting
- Follow Rust naming conventions

### TypeScript/React

- Use `npm run format` for formatting
- Use `npm run lint` for linting
- Follow React best practices

## Useful Commands

```bash
# Development
npm run tauri dev              # Start dev server
npm run dev                    # Start Vite only
cargo check                    # Check Rust code
cargo build                    # Build Rust

# Testing
cargo test                     # Run Rust tests
npm test                       # Run React tests (future)

# Code Quality
cargo fmt                      # Format Rust
cargo clippy                   # Lint Rust
npm run format                 # Format TypeScript
npm run lint                   # Lint TypeScript

# Building
npm run tauri build            # Build production app
npm run build                  # Build React only

# Cleaning
cargo clean                    # Clean Rust build
rm -rf node_modules dist       # Clean Node
```

## Environment Variables

### Development

- `RUST_BACKTRACE=1` - Show Rust backtraces
- `RUST_LOG=debug` - Enable debug logging

### Production

- `TAURI_PRIVATE_KEY` - Signing key for updates
- `TAURI_KEY_PASSWORD` - Password for signing key

## Deployment

### Development Build

```bash
npm run tauri dev
```

### Production Build

```bash
npm run tauri build
```

Creates installers in `src-tauri/target/release/bundle/`:
- Windows: `.msi`
- macOS: `.dmg`
- Linux: `.AppImage` or `.deb`

### Signing (macOS/Windows)

See Tauri documentation for code signing setup.

## Troubleshooting

### "Port 1420 already in use"

```bash
# Kill process using port
# Windows
netstat -ano | findstr :1420
taskkill /PID <PID> /F

# macOS/Linux
lsof -i :1420
kill -9 <PID>
```

### "Rust compilation errors"

```bash
# Clean and rebuild
cargo clean
cargo build
```

### "Module not found" in React

- Check import path uses `@/` alias
- Verify file exists
- Check tsconfig.json has path alias

### "Tauri command not found"

- Verify command is in `generate_handler!` macro
- Check command name matches exactly
- Restart dev server

## Resources

- **Tauri Docs**: https://tauri.app/
- **React Docs**: https://react.dev/
- **Rust Book**: https://doc.rust-lang.org/book/
- **sqlx Docs**: https://github.com/launchbadge/sqlx
- **Tokio Docs**: https://tokio.rs/

## Getting Help

1. Check existing documentation
2. Search GitHub issues
3. Check Tauri Discord
4. Review code comments
5. Ask in project discussions

Happy coding! 🚀
