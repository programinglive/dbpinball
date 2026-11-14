---
layout: home

hero:
  name: DBPinball
  text: A CLI-first database workspace for MySQL and PostgreSQL
  tagline: Inspect databases from the terminal, or from a desktop app that shares the same Rust core and the same connection store.
  actions:
    - theme: brand
      text: Get started
      link: /guide/
    - theme: alt
      text: CLI reference
      link: /reference/cli
    - theme: alt
      text: Architecture
      link: /architecture/

features:
  - title: One core, two front ends
    details: The Rust core owns connection storage, validation and inspection. The CLI and the Tauri desktop are thin adapters over it, so they cannot drift apart.
  - title: Credentials stay in the OS keyring
    details: Passwords are stored in Keychain, Credential Manager or libsecret. The connection store holds metadata only and has no password field.
  - title: Safe concurrent edits
    details: A cross-process lock and a monotonic revision stop the desktop and the CLI from silently overwriting each other's profiles.
  - title: Honest about limits
    details: Safe Sync, the security policy milestone and several desktop pages are not available yet. The roadmap says exactly what is implemented and what is not.
---

## What DBPinball is

DBPinball is a database workspace for developers and administrators who work with
MySQL and PostgreSQL. It has two front ends over one Rust core:

- a **standalone CLI** (`dbpinball`) for scripted and headless inspection;
- a **desktop application** (Tauri + React) for interactive work.

Both read and write the same connection store, so a profile created in one
appears in the other.

## What is available today

| Capability | Status |
| --- | --- |
| Connection vault (create, edit, test, delete) | <span class="status status-available">Available</span> |
| Credential storage in the OS keyring | <span class="status status-available">Available</span> |
| Database and table listing | <span class="status status-available">Available</span> |
| Query execution in the desktop workspace | <span class="status status-available">Available</span> |
| `dbpinball` CLI (`doctor`, `connections`, `schema inspect`) | <span class="status status-available">Available</span> |
| Shared connection store between desktop and CLI | <span class="status status-available">Available</span> |
| Backup, restore and copy from the desktop UI | <span class="status status-planned">Not routed in the UI</span> |
| Shared security policy and destination verification (M3) | <span class="status status-available">Available</span> |
| Safe Sync (M4 and later) | <span class="status status-unavailable">Not available</span> |

See the [roadmap](/roadmap) for the full picture.

## Where to go next

- New to the project? Start with [Requirements](/guide/requirements) and
  [Installation](/guide/installation).
- Want the terminal? Read [CLI basics](/guide/cli) and the
  [CLI reference](/reference/cli).
- Want the desktop? Read [Desktop basics](/guide/desktop).
- Integrating or contributing? Read the [architecture](/architecture/) and
  [development](/development/) sections.
