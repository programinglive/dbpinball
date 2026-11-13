# Getting started

DBPinball is a database workspace for MySQL and PostgreSQL. This section takes
you from a clean machine to a working connection, in the terminal or in the
desktop app.

## The short version

1. Check the [requirements](/guide/requirements).
2. Build from source with [installation](/guide/installation).
3. Follow the [quick start](/guide/quick-start).
4. Create your [first connection](/guide/first-connection).

## Two front ends, one store

The CLI and the desktop share one connection store at
`dirs::config_dir()/dbpinball/connections.json` and one OS keyring entry per
profile. A profile created in the CLI appears in the desktop, and the other way
around. You do not need to keep two sets of profiles.

## What this documentation covers

| Section | What it answers |
| --- | --- |
| [Guide](/guide/) | How to install and use DBPinball |
| [Architecture](/architecture/) | How the core, the CLI, the desktop and the store fit together |
| [Security](/security/) | How credentials are stored, and what the limits are |
| [Reference](/reference/cli) | Exact commands, configuration and error behaviour |
| [Development](/development/) | How to build, test and contribute |
| [Roadmap](/roadmap) | What is implemented, what is planned, what is not available |

::: warning Not a marketing page
Every capability described here is taken from the repository and its tests. If a
feature is planned or incomplete, it is labelled as such.
:::
