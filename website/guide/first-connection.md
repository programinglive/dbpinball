# First connection

A connection profile is metadata plus a credential. The metadata lives in the
shared store; the credential lives in the OS keyring under the profile id.

## Create a profile

### From the CLI

```bash
dbpinball connections add \
  --name analytics \
  --engine postgres \
  --host db.example.internal \
  --port 5432 \
  --database analytics \
  --username analyst \
  --target staging
```

`--target` is a user hint (`production`, `staging`, `local`, `unknown`). It is
metadata, not proof of where the database lives, and it is never used on its own
to decide that a database is safe to write to.

### From the desktop

1. Launch the app.
2. Use the **+** toolbar button and choose **Add MySQL Connection** or
   **Add PostgreSQL Connection**.
3. Enter host, port, database, username and password.
4. Test the connection, then save it.

## What gets stored

| Field | Stored in |
| --- | --- |
| `id`, `name`, `engine`, `host`, `port`, `database`, `username`, `useSsl`, `target` | `connections.json` |
| Password | OS keyring, service `dbpinball`, account = `id` |

The store has no password field. A test asserts the key is absent.

## Test the connection

```bash
dbpinball connections test <id>
```

The CLI reads the password from the keyring by default. With
`--credential-source env` it reads `DBPINBALL_PASSWORD` instead; that source
cannot store or delete passwords.

## Change the password

```bash
dbpinball connections set-password <id>
```

The password is read from stdin. The profile metadata is unchanged.

## Remove the profile

```bash
dbpinball connections remove <id> --yes
```

Removal deletes the credential first, then the metadata. If the credential
cannot be deleted, the profile stays so it never looks removed while a
credential is still stored. Removing a profile never deletes a database or its
data.

## Next

- [Managing connections](/guide/connections)
- [Credentials and keyring](/guide/credentials)
- [Troubleshooting](/guide/troubleshooting)
