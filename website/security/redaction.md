# Redaction guarantees

Every error message passes through `redact` before it reaches a user.

## What is removed

`redact` removes the userinfo from any URL-shaped text:

```text
postgresql://app:super-secret@127.0.0.1:5432/app
        ↓
postgresql://***:***@127.0.0.1:5432/app
```

It handles an `@` inside a password, and it leaves plain text untouched.

## Where it applies

- `CoreError::redacted` builds an error whose message has been redacted.
- The desktop adapter maps error codes to fixed messages, so a raw core message
  never reaches the UI.
- The CLI prints the redacted message.

## What is not claimed

Redaction removes credentials from URL-shaped text. It does not remove every
possible secret from arbitrary text. Do not put a secret in a profile name, a
database name, or a host field.

## Tests

- `redaction_removes_password`
- `redaction_handles_at_sign_in_password`
- `redaction_leaves_plain_text_untouched`
- `redacted_error_never_contains_the_secret`
- `safe_error_never_leaks_a_credential`
- `safe_error_does_not_leak_a_filesystem_path`

## Related

- [Credential storage](/security/credential-storage)
- [Error behaviour](/reference/errors)
