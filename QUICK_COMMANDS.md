# Quick Commands Reference

## Development

```bash
# Start dev server
npm run tauri:dev

# Build frontend
npm run build

# Type check
npm run lint
```

## Git & Commits

```bash
# Stage changes
git add .

# Commit (conventional format)
git commit -m 'type(scope): description'

# Examples:
git commit -m 'feat(backup): add compression'
git commit -m 'fix(restore): handle timeout'
git commit -m 'docs: update README'

# Push to GitHub
git push origin main

# View commits
git log --oneline
```

## Releases

```bash
# Auto-detect version bump
npm run release

# Specific version
npm run release:major
npm run release:minor
npm run release:patch

# Push release
git push origin main --tags
```

## Building for Production

```bash
# Build desktop app
npm run tauri:build

# Output: src-tauri/target/release/bundle/
# - Windows: .msi installer
# - macOS: .dmg bundle
# - Linux: .AppImage or .deb
```

## Database Testing

```bash
# MySQL (if installed locally)
mysql -u root -p

# PostgreSQL (if installed locally)
psql -U postgres
```

## Useful Git Commands

```bash
# Check status
git status

# View changes
git diff

# View commits
git log --oneline -10

# Create branch
git checkout -b feature/name

# Switch branch
git checkout main

# Delete branch
git branch -d feature/name

# Undo last commit (keep changes)
git reset --soft HEAD~1

# Undo last commit (discard changes)
git reset --hard HEAD~1
```

## Troubleshooting

```bash
# Clean build
cd src-tauri && cargo clean
rm -rf node_modules dist

# Reinstall dependencies
npm install

# Check Rust compilation
cd src-tauri && cargo check

# Run Rust tests
cd src-tauri && cargo test

# Kill port 1420 (dev server)
# Windows:
netstat -ano | findstr :1420
taskkill /PID <PID> /F

# macOS/Linux:
lsof -i :1420
kill -9 <PID>
```

## Commit Types Quick Reference

| Type | Purpose | Example |
|------|---------|---------|
| feat | New feature | `feat(backup): add compression` |
| fix | Bug fix | `fix(restore): handle timeout` |
| docs | Documentation | `docs: update README` |
| style | Code formatting | `style: format code` |
| refactor | Code refactoring | `refactor(copy): improve perf` |
| test | Tests | `test: add backup tests` |
| build | Build system | `build: update dependencies` |
| ci | CI/CD | `ci: add github actions` |
| perf | Performance | `perf(backup): optimize speed` |
| chore | Maintenance | `chore: update deps` |

## Project Structure

```
d:\code\dbpinball/
├── src/                    # React frontend
├── src-tauri/              # Rust backend
├── README.md               # User guide
├── QUICKSTART.md           # 5-min setup
├── DEVELOPMENT.md          # Dev guide
├── COMMITER_SETUP.md       # Commit guide
└── GITHUB_SETUP_SUMMARY.md # GitHub setup
```

## Important URLs

- **GitHub**: https://github.com/programinglive/dbpinball
- **NPM Package**: https://www.npmjs.com/package/@programinglive/commiter
- **Tauri Docs**: https://tauri.app/
- **React Docs**: https://react.dev/

## Status

✅ Project ready for development
✅ GitHub connected
✅ Commiter configured
✅ Dev server running on localhost:1420

Start coding: `npm run tauri:dev`
