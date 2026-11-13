# Quick start

This walkthrough creates one connection and inspects a database, first with the
CLI and then in the desktop.

## 1. Check the environment

```bash
dbpinball doctor
```

`doctor` reports the local environment. Add `--json` for a machine-readable
object.

## 2. Create a connection

The password is read from stdin. There is no `--password` flag, so a password
never appears in shell history or process arguments.

```bash
dbpinball connections add \
  --name local \
  --engine postgres \
  --host 127.0.0.1 \
  --port 5432 \
  --database app \
  --username postgres \
  --target local
```

You are prompted for the password. In a non-interactive shell, pipe exactly one
line:

```bash
printf '%s\n' "$DB_PASSWORD" | dbpinball connections add \
  --name local --engine postgres --host 127.0.0.1 --port 5432 \
  --database app --username postgres --target local
```

## 3. List and test it

```bash
dbpinball connections list
dbpinball connections test <id>
```

`connections list` never prints passwords.

## 4. Inspect the schema

```bash
# List databases on the server
dbpinball schema inspect --connection <id>

# List tables in one database
dbpinball schema inspect --connection <id> --database app
```

## 5. Open the desktop

```bash
npm run tauri dev
```

The profile you created in the CLI is already there, because both front ends use
the same store. Double-click it to open the workspace.

## 6. Remove it when you are done

```bash
dbpinball connections remove <id> --yes
```

Removal deletes the profile and its keyring credential. It never touches the
database itself.

## Next steps

- [Managing connections](/guide/connections)
- [Credentials and keyring](/guide/credentials)
- [CLI basics](/guide/cli)
- [Desktop basics](/guide/desktop)
