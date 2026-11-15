# Support

## Documentation first

Most questions are answered in the documentation:

- **Installation:** <https://dbpinball.programinglive.com/guide/installation>
- **Quick start:** <https://dbpinball.programinglive.com/guide/quick-start>
- **CLI reference:** <https://dbpinball.programinglive.com/reference/cli>
- **Safe Sync design and limits:** <https://dbpinball.programinglive.com/security/safe-sync-design>
- **Troubleshooting:** <https://dbpinball.programinglive.com/guide/troubleshooting>

`dbpinball doctor` reports the version, the configuration directory, the
credential store and whether the optional `mysqldump`/`pg_dump` tools are on
`PATH`. It never prints a password.

## Installation questions

If an installer or the CLI does not work, check the
[installation guide](https://dbpinball.programinglive.com/guide/installation)
first. It covers the `'dbpinball' is not recognized` case, SmartScreen warnings
and antivirus false positives.

If the guide does not help, open a
[bug report](https://github.com/programinglive/dbpinball/issues/new?template=bug_report.yml)
and include the version from `dbpinball --version`.

## Bug reports

Use the [bug report template](https://github.com/programinglive/dbpinball/issues/new?template=bug_report.yml).
Include the version, the engine and its version, your operating system, and
reproduction steps that use **synthetic** data.

Do not attach real dumps, connection strings or customer records.

## Feature requests

Use the
[feature request template](https://github.com/programinglive/dbpinball/issues/new?template=feature_request.yml).
Check the README's known limitations first: some capabilities are deliberately
refused rather than unimplemented by accident.

## Security vulnerabilities

**Do not open a public issue.** Use GitHub private vulnerability reporting:

<https://github.com/programinglive/dbpinball/security/advisories/new>

See [SECURITY.md](SECURITY.md) for what to include and what not to send.

## Documentation problems

Documentation lives in `website/` and is published from `main`. If a page is
wrong or a command does not work, open an issue or a pull request that fixes it.
Corrections are welcome and do not need a prior discussion.

## What this project does not offer

- No commercial support contract.
- No private help channel for general questions.
- No guarantee of a response time. This is a volunteer-maintained project.
