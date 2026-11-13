# Local setup

## Prerequisites

- Node.js 20.19+ or 22.12+
- Rust 1.77.2+
- Platform build dependencies for Tauri

## Clone and install

```bash
git clone https://github.com/programinglive/dbpinball.git
cd dbpinball
npm install
```

## Run the desktop

```bash
npm run tauri dev
```

## Build the CLI

```bash
cargo build -p dbpinball-cli
./target/debug/dbpinball doctor
```

## Run the docs site

```bash
cd website
npm install
npm run dev
```

## Related

- [Repository structure](/development/repository-structure)
- [Testing](/development/testing)
