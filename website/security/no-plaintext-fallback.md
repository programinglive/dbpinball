# No plaintext fallback

If the OS keyring is unavailable, DBPinball fails. It does not fall back to a
plaintext file, an environment variable, or a prompt that stores the password
somewhere else.

## What happens

| Situation | Behaviour |
| --- | --- |
| Keyring unavailable | Exit code 3; the command fails |
| Keyring locked | Exit code 3; the command fails |
| Credential missing | Exit code 3; the profile remains |
| Permission denied | Exit code 3; the profile remains |

## The one exception, and why it is not a fallback

The CLI can read a password from `DBPINBALL_PASSWORD` when you explicitly pass
`--credential-source env`. That source:

- is per-invocation;
- cannot store a password;
- cannot delete a password;
- is never used automatically.

It is a deliberate opt-in for CI and automation, not a fallback the tool chooses
for you.

## Related

- [Credentials and keyring](/guide/credentials)
- [Limitations](/security/limitations)
