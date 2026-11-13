# Agent instructions — DBPinball

## Repository identity gate

This repository is **programinglive/dbpinball** — a Rust workspace (core library
+ standalone CLI + desktop app). It is a separate open-source project from any
private repository on the machine that runs the agent, and from any workflow
tooling that orchestrates those repositories.

Before the first write in a task, confirm all of the following:

```bash
git rev-parse --show-toplevel     # must be the DBPinball checkout
git remote get-url origin         # must be programinglive/dbpinball
git branch --show-current         # must match the branch the task declares
```

If any of these disagree with the task you were given — or the task describes
work that belongs to another repository — **stop**. Do not edit, commit, push, or
merge. Report the mismatch and ask for explicit authorization.

A task description is not proof of which repository the work belongs to. Name the
repository in the task, or verify it with the commands above.

## Milestones

| Milestone | Scope | State |
|---|---|---|
| M1 | Headless Rust core extraction | merged |
| M2 | Standalone `dbpinball` CLI foundation | merged (PR #3) |
| M2.1 | Desktop connection store migration | next |
| M3 | Shared security policy and destination verification | planned |
| M4 | PostgreSQL safe sync | planned |

Work stays inside the milestone the task names. The approved plans live in
`docs/superpowers/plans/`.

## Workflow

Cross-project routing, focus binding and identity enforcement are owned by the
orchestration layer (`dev-workflow` MCP), not by this repository. See
`docs/repository-identity.md` in that server for the rule and the
`devwf identity` preflight command. Do not duplicate that logic here.
