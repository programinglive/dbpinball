## Summary

<!-- What changed and why. One concern per pull request. -->

## Related issue

<!-- Closes #123, or "none". -->

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor
- [ ] Documentation
- [ ] Build or tooling
- [ ] Security

## Testing

<!-- Commands you ran and their result. Paste the actual output, not a claim. -->

```text
cargo test --workspace
npm test
```

## Safety impact

<!--
Does this touch credentials, connection URLs, SQL execution, masking, plan
authorization, destination verification, or anything that writes to a database?
If yes, describe the threat addressed and the test that proves the control.
If no, write "none".
-->

## Documentation impact

<!-- Which pages or files changed, or why none were needed. -->

## Breaking changes

<!-- Command names, payload shapes, stored data, or plan format. Write "none" if there are none. -->

## Checklist

- [ ] Tests added or updated for behaviour changes
- [ ] Documentation updated where behaviour, commands, or capability states changed
- [ ] No real credentials, connection strings, or production data included
- [ ] Existing Tauri command names and payload shapes preserved
- [ ] No unrelated formatting or refactoring
- [ ] No generated artifacts or build output committed
