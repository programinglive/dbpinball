# @programinglive/commiter Setup Guide

> **Historical record.** The installed version is now `@programinglive/commiter@^1.5.1`, and releases run through `commiter release`. The `scripts/release.js` path described below no longer exists. See `package.json` for the current scripts.

## Overview

dbpinball uses `@programinglive/commiter` for standardized commit messages, changelog generation, and semantic versioning. This ensures clean, professional release management.

## What's Installed

✅ **@programinglive/commiter** v1.1.6
✅ **Husky** - Git hooks management
✅ **commitlint** - Commit message validation
✅ **standard-version** - Semantic versioning

## Commit Format

All commits must follow the conventional commits format:

```
type(scope): subject

body (optional)

footer (optional)
```

### Types

- **feat**: A new feature
- **fix**: A bug fix
- **perf**: Performance improvement
- **refactor**: Code refactoring
- **docs**: Documentation changes
- **style**: Code style changes (formatting, etc.)
- **test**: Adding or updating tests
- **build**: Build system changes
- **ci**: CI/CD changes
- **chore**: Maintenance tasks
- **revert**: Reverting a previous commit

### Examples

```bash
# Feature
git commit -m 'feat(backup): add compression support'

# Bug fix
git commit -m 'fix(restore): handle connection timeout'

# Documentation
git commit -m 'docs: update README with examples'

# Multiple lines
git commit -m 'feat(copy): add cross-engine support

- MySQL to PostgreSQL type mapping
- PostgreSQL to MySQL type mapping
- Handles ENUM, SPATIAL, and UNSIGNED types'
```

## Husky Hooks

Husky automatically validates commits before they're created:

### Pre-commit Hook
- Runs before commit
- Validates code quality
- Can prevent commits with issues

### Commit-msg Hook
- Validates commit message format
- Ensures conventional commits format
- Rejects non-compliant messages

## Making Commits

### Standard Commit

```bash
git add .
git commit -m 'feat(feature-name): description'
```

### Interactive Commit (Recommended)

```bash
git add .
git commit
# Opens editor for detailed message
```

### Bypass Hooks (Not Recommended)

```bash
git commit --no-verify
```

## Release Management

### Automatic Version Bumping

The commiter automatically determines version bumps based on commits:

- **Major** (1.0.0 → 2.0.0): Breaking changes
- **Minor** (1.0.0 → 1.1.0): New features
- **Patch** (1.0.0 → 1.0.1): Bug fixes

### Create a Release

```bash
# Auto-detect version bump
npm run release

# Specific version bump
npm run release:major
npm run release:minor
npm run release:patch

# Pre-release
npm run release -- --prerelease beta
```

### What Happens During Release

1. ✅ Analyzes commits since last release
2. ✅ Determines version bump
3. ✅ Generates CHANGELOG.md
4. ✅ Updates package.json version
5. ✅ Creates git tag
6. ✅ Commits release changes
7. ✅ Ready to push to GitHub

### Release Example

```bash
# Current version: 0.1.0
# Commits since last release:
#   - feat(backup): add compression
#   - fix(restore): handle timeout
#   - docs: update README

npm run release
# → Detects 1 feature + 1 fix = minor bump
# → Version becomes 0.2.0
# → CHANGELOG.md updated
# → Git tag v0.2.0 created
# → Ready to push
```

## Changelog Generation

The `CHANGELOG.md` is automatically generated with:

- ✨ Features
- 🐛 Bug Fixes
- ⚡ Performance improvements
- ♻️ Refactors
- 📝 Documentation
- 💄 Styles
- ✅ Tests
- 🏗️ Build System
- 👷 CI/CD
- 🧹 Chores
- ⏪ Reverts

Example:

```markdown
# Changelog

## [0.2.0](https://github.com/programinglive/dbpinball/compare/v0.1.0...v0.2.0) (2024-01-15)

### ✨ Features

* **backup**: add compression support ([abc1234](https://github.com/programinglive/dbpinball/commit/abc1234))

### 🐛 Bug Fixes

* **restore**: handle connection timeout ([def5678](https://github.com/programinglive/dbpinball/commit/def5678))
```

## Configuration Files

### commitlint.config.js

Validates commit message format:

```javascript
module.exports = {
  extends: ['@commitlint/config-conventional'],
  rules: {
    'type-enum': [
      2,
      'always',
      [
        'feat',
        'fix',
        'perf',
        'refactor',
        'docs',
        'style',
        'test',
        'build',
        'ci',
        'chore',
        'revert',
      ],
    ],
  },
};
```

### .husky/pre-commit

Runs before each commit (can add linting, tests, etc.)

### .husky/commit-msg

Validates commit message format

### package.json (standard-version config)

```json
{
  "standard-version": {
    "releaseCommitMessageFormat": "chore(release): {{currentTag}} 🚀",
    "types": [
      { "type": "feat", "section": "✨ Features" },
      { "type": "fix", "section": "🐛 Bug Fixes" },
      ...
    ]
  }
}
```

## Workflow Example

### Day-to-day Development

```bash
# 1. Make changes
# ... edit files ...

# 2. Stage changes
git add .

# 3. Commit with conventional format
git commit -m 'feat(backup): add compression support'

# 4. Push to branch
git push origin feature-branch
```

### Release Process

```bash
# 1. Ensure all commits are pushed
git push origin main

# 2. Create release
npm run release

# 3. Push release tag and commits
git push origin main --tags

# 4. Create GitHub release (optional)
# Go to https://github.com/programinglive/dbpinball/releases
# Create release from tag with changelog
```

## Troubleshooting

### "Commit message validation failed"

**Problem**: Commit message doesn't follow conventional format

**Solution**:
```bash
# Use correct format
git commit -m 'type(scope): description'

# Examples:
git commit -m 'feat(backup): add compression'
git commit -m 'fix(restore): handle timeout'
git commit -m 'docs: update README'
```

### "Husky hook failed"

**Problem**: Pre-commit hook failed

**Solution**:
```bash
# Check what failed
git status

# Fix issues
# ... fix code ...

# Try again
git add .
git commit -m 'type(scope): description'
```

### "Cannot create release"

**Problem**: Release creation fails

**Solution**:
```bash
# Ensure all changes are committed
git status

# Ensure you're on main branch
git branch

# Try again
npm run release
```

## Best Practices

1. **Commit Often** - Small, focused commits are easier to review
2. **Use Scopes** - Specify what part of the app changed (backup, restore, etc.)
3. **Clear Messages** - Be descriptive in commit messages
4. **Follow Format** - Always use conventional commits format
5. **Review Before Commit** - Check what you're committing
6. **Use Branches** - Create feature branches for new work
7. **Squash if Needed** - Combine related commits before merging

## Integration with GitHub

### Automatic Releases

Set up GitHub Actions to automatically:
1. Run tests on PR
2. Create releases on merge to main
3. Publish to npm
4. Create GitHub releases

### Branch Protection

Recommended rules for `main` branch:
- ✅ Require pull request reviews
- ✅ Require status checks to pass
- ✅ Require branches to be up to date
- ✅ Require commit message validation

## Resources

- [Conventional Commits](https://www.conventionalcommits.org/)
- [Husky Docs](https://typicode.github.io/husky/)
- [commitlint Docs](https://commitlint.js.org/)
- [standard-version Docs](https://github.com/conventional-changelog/standard-version)
- [@programinglive/commiter](https://github.com/programinglive/commiter)

## Quick Reference

```bash
# Development
npm run tauri:dev              # Start dev server
npm run build                  # Build frontend
npm run lint                   # Check types

# Commits
git commit -m 'feat(scope): description'
git commit -m 'fix(scope): description'
git commit -m 'docs: description'

# Releases
npm run release                # Auto-detect version
npm run release:major          # Major release
npm run release:minor          # Minor release
npm run release:patch          # Patch release

# Push
git push origin main --tags    # Push with tags
```

Happy committing! 🎉
