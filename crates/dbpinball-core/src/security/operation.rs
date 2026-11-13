//! Database operation classes and SQL classification.
//!
//! WHY THIS EXISTS
//! ---------------
//! A connection profile that says `local` is a user label, not proof that a
//! database is safe to write. Before any policy can decide, it has to know what
//! kind of operation is being attempted. This module is that vocabulary.
//!
//! SQL classification is deliberately conservative. It is a *risk signal*, not a
//! security boundary: a statement classified as read-only is still executed
//! under the database's own permissions, and a statement classified as a write
//! is never allowed to skip the destination policy. Anything the classifier does
//! not understand is `Unknown`, and `Unknown` is treated as a write.

use serde::{Deserialize, Serialize};

/// What a database operation is allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseOperation {
    /// Listing databases, tables, or connection tests.
    ReadOnlyInspection,
    /// A query the caller believes is read-only.
    ReadOnlyQuery,
    /// Dumping a database to a file.
    BackupExport,
    /// Loading a file into a local database.
    LocalDataImport,
    /// Restoring a dump into a database.
    DatabaseRestore,
    /// Copying one database into another.
    DatabaseCopy,
    /// Creating, altering, or dropping schema objects.
    SchemaMutation,
    /// Inserting, updating, or deleting rows.
    DataMutation,
    /// Preparing a sanitized copy of production data (M4).
    SafeSyncPrepare,
    /// Executing a sanitized copy into a development target (M4).
    SafeSyncExecute,
}

impl DatabaseOperation {
    /// True when the operation can change a database.
    ///
    /// `SafeSyncPrepare` reads a protected source, so it is not a write to the
    /// source, but it is still a dangerous operation that needs a verified
    /// source and destination pair.
    pub fn is_write(self) -> bool {
        matches!(
            self,
            DatabaseOperation::LocalDataImport
                | DatabaseOperation::DatabaseRestore
                | DatabaseOperation::DatabaseCopy
                | DatabaseOperation::SchemaMutation
                | DatabaseOperation::DataMutation
                | DatabaseOperation::SafeSyncExecute
        )
    }

    /// True when the operation reads a protected source.
    pub fn touches_source(self) -> bool {
        matches!(self, DatabaseOperation::SafeSyncPrepare)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DatabaseOperation::ReadOnlyInspection => "read_only_inspection",
            DatabaseOperation::ReadOnlyQuery => "read_only_query",
            DatabaseOperation::BackupExport => "backup_export",
            DatabaseOperation::LocalDataImport => "local_data_import",
            DatabaseOperation::DatabaseRestore => "database_restore",
            DatabaseOperation::DatabaseCopy => "database_copy",
            DatabaseOperation::SchemaMutation => "schema_mutation",
            DatabaseOperation::DataMutation => "data_mutation",
            DatabaseOperation::SafeSyncPrepare => "safe_sync_prepare",
            DatabaseOperation::SafeSyncExecute => "safe_sync_execute",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
        Some(match normalized.as_str() {
            "read_only_inspection" | "inspect" => DatabaseOperation::ReadOnlyInspection,
            "read_only_query" | "query" => DatabaseOperation::ReadOnlyQuery,
            "backup_export" | "backup" => DatabaseOperation::BackupExport,
            "local_data_import" | "import" => DatabaseOperation::LocalDataImport,
            "database_restore" | "restore" => DatabaseOperation::DatabaseRestore,
            "database_copy" | "copy" => DatabaseOperation::DatabaseCopy,
            "schema_mutation" | "schema" => DatabaseOperation::SchemaMutation,
            "data_mutation" | "mutation" => DatabaseOperation::DataMutation,
            "safe_sync_prepare" => DatabaseOperation::SafeSyncPrepare,
            "safe_sync_execute" => DatabaseOperation::SafeSyncExecute,
            _ => return None,
        })
    }
}

/// The classifier's verdict for one SQL text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SqlClass {
    /// Every statement is provably read-only.
    ReadOnly,
    /// At least one statement can write.
    Write,
    /// The classifier cannot prove either way.
    Unknown,
}

impl SqlClass {
    /// The operation a caller should be authorized for.
    pub fn operation(self) -> DatabaseOperation {
        match self {
            SqlClass::ReadOnly => DatabaseOperation::ReadOnlyQuery,
            // Unknown is treated as a write: fail closed.
            SqlClass::Write | SqlClass::Unknown => DatabaseOperation::DataMutation,
        }
    }
}

/// Keywords that can only read.
const READ_KEYWORDS: &[&str] = &["select", "show", "describe", "desc", "values", "table"];

/// Keywords that can write or change session state.
const WRITE_KEYWORDS: &[&str] = &[
    "insert",
    "update",
    "delete",
    "merge",
    "create",
    "alter",
    "drop",
    "truncate",
    "grant",
    "revoke",
    "call",
    "do",
    "execute",
    "copy",
    "load",
    "replace",
    "rename",
    "comment",
    "vacuum",
    "analyze",
    "reindex",
    "cluster",
    "set",
    "reset",
    "begin",
    "commit",
    "rollback",
    "savepoint",
    "lock",
    "prepare",
    "deallocate",
    "discard",
    "listen",
    "notify",
    "unlisten",
    "refresh",
    "import",
    "security",
    "attach",
    "detach",
    "pragma",
    "use",
    "kill",
    "flush",
    "optimize",
    "repair",
    "install",
    "uninstall",
    "handler",
    "shutdown",
];

/// Keywords that may execute arbitrary code and therefore cannot be proven safe.
const OPAQUE_KEYWORDS: &[&str] = &["call", "do", "execute", "prepare"];

/// Classify a SQL text.
///
/// The classifier strips comments and string literals first, so a `;` or a
/// keyword inside a literal cannot change the verdict. It then splits on `;`
/// and classifies every statement; the strictest verdict wins.
pub fn classify_sql(sql: &str) -> SqlClass {
    let stripped = strip_comments_and_literals(sql);
    let statements: Vec<&str> = stripped
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .collect();

    if statements.is_empty() {
        return SqlClass::Unknown;
    }

    let mut verdict = SqlClass::ReadOnly;

    for statement in statements {
        let class = classify_statement(statement);
        verdict = strictest(verdict, class);
        if verdict == SqlClass::Write {
            return SqlClass::Write;
        }
    }

    verdict
}

fn strictest(left: SqlClass, right: SqlClass) -> SqlClass {
    use SqlClass::*;
    match (left, right) {
        (Write, _) | (_, Write) => Write,
        (Unknown, _) | (_, Unknown) => Unknown,
        _ => ReadOnly,
    }
}

fn classify_statement(statement: &str) -> SqlClass {
    let lower = statement.to_ascii_lowercase();
    let first = lower
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_ascii_alphanumeric());

    if first.is_empty() {
        return SqlClass::Unknown;
    }

    if OPAQUE_KEYWORDS.contains(&first) {
        return SqlClass::Unknown;
    }

    if first == "with" {
        // A CTE can contain INSERT/UPDATE/DELETE/MERGE. Scan the whole
        // statement rather than trusting the leading WITH.
        if contains_write_keyword(&lower) {
            return SqlClass::Write;
        }
        return SqlClass::ReadOnly;
    }

    if first == "explain" {
        // EXPLAIN ANALYZE executes the statement.
        if lower.contains("analyze") {
            return SqlClass::Unknown;
        }
        return SqlClass::ReadOnly;
    }

    if first == "select" {
        // SELECT ... INTO creates a table in PostgreSQL.
        if contains_word(&lower, "into") {
            return SqlClass::Write;
        }
        return SqlClass::ReadOnly;
    }

    if WRITE_KEYWORDS.contains(&first) {
        return SqlClass::Write;
    }

    if READ_KEYWORDS.contains(&first) {
        return SqlClass::ReadOnly;
    }

    SqlClass::Unknown
}

fn contains_write_keyword(lower: &str) -> bool {
    WRITE_KEYWORDS
        .iter()
        .any(|keyword| contains_word(lower, keyword))
}

fn contains_word(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut start = 0;

    while let Some(found) = haystack[start..].find(needle) {
        let index = start + found;
        let before_ok = index == 0 || !is_word_byte(bytes[index - 1]);
        let after = index + needle.len();
        let after_ok = after >= bytes.len() || !is_word_byte(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
        start = index + needle.len();
    }

    false
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Remove `--` and `/* */` comments and quoted literals.
///
/// The result keeps the statement structure but cannot be tricked by a keyword
/// or a semicolon inside a string.
fn strip_comments_and_literals(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut output = String::with_capacity(sql.len());
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];

        // Line comment.
        if byte == b'-' && index + 1 < bytes.len() && bytes[index + 1] == b'-' {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            output.push(' ');
            continue;
        }

        // Block comment.
        if byte == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'*' {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            output.push(' ');
            continue;
        }

        // Single-quoted literal, with '' escaping.
        if byte == b'\'' {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\'' {
                    if index + 1 < bytes.len() && bytes[index + 1] == b'\'' {
                        index += 2;
                        continue;
                    }
                    index += 1;
                    break;
                }
                index += 1;
            }
            output.push_str("''");
            continue;
        }

        // Double-quoted identifier.
        if byte == b'"' {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'"' {
                    index += 1;
                    break;
                }
                index += 1;
            }
            output.push_str("\"\"");
            continue;
        }

        output.push(byte as char);
        index += 1;
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_select_is_read_only() {
        assert_eq!(classify_sql("SELECT * FROM users"), SqlClass::ReadOnly);
    }

    #[test]
    fn insert_is_a_write() {
        assert_eq!(classify_sql("INSERT INTO t VALUES (1)"), SqlClass::Write);
    }

    #[test]
    fn a_writable_cte_is_a_write() {
        let sql = "WITH moved AS (DELETE FROM t RETURNING *) SELECT * FROM moved";
        assert_eq!(classify_sql(sql), SqlClass::Write);
    }

    #[test]
    fn a_read_only_cte_is_read_only() {
        let sql = "WITH recent AS (SELECT * FROM t) SELECT * FROM recent";
        assert_eq!(classify_sql(sql), SqlClass::ReadOnly);
    }

    #[test]
    fn multiple_statements_take_the_strictest_verdict() {
        let sql = "SELECT 1; DROP TABLE users";
        assert_eq!(classify_sql(sql), SqlClass::Write);
    }

    #[test]
    fn select_into_is_a_write() {
        assert_eq!(
            classify_sql("SELECT * INTO backup_table FROM users"),
            SqlClass::Write
        );
    }

    #[test]
    fn explain_analyze_is_not_proven_read_only() {
        assert_eq!(
            classify_sql("EXPLAIN ANALYZE DELETE FROM t"),
            SqlClass::Unknown
        );
    }

    #[test]
    fn a_stored_procedure_call_is_opaque() {
        assert_eq!(classify_sql("CALL do_something()"), SqlClass::Unknown);
    }

    #[test]
    fn a_keyword_inside_a_literal_does_not_change_the_verdict() {
        assert_eq!(
            classify_sql("SELECT 'DROP TABLE users' AS note"),
            SqlClass::ReadOnly
        );
    }

    #[test]
    fn a_semicolon_inside_a_literal_does_not_split_statements() {
        assert_eq!(classify_sql("SELECT 'a;b' AS value"), SqlClass::ReadOnly);
    }

    #[test]
    fn a_comment_cannot_hide_a_write() {
        assert_eq!(classify_sql("/* SELECT */ DELETE FROM t"), SqlClass::Write);
    }

    #[test]
    fn an_unknown_statement_fails_closed() {
        assert_eq!(classify_sql("SOMETHING NEW"), SqlClass::Unknown);
        assert_eq!(classify_sql(""), SqlClass::Unknown);
    }

    #[test]
    fn unknown_maps_to_a_write_operation() {
        assert_eq!(
            SqlClass::Unknown.operation(),
            DatabaseOperation::DataMutation
        );
        assert_eq!(
            SqlClass::ReadOnly.operation(),
            DatabaseOperation::ReadOnlyQuery
        );
    }

    #[test]
    fn operation_round_trips_through_its_name() {
        for operation in [
            DatabaseOperation::ReadOnlyInspection,
            DatabaseOperation::DatabaseRestore,
            DatabaseOperation::SafeSyncExecute,
        ] {
            assert_eq!(
                DatabaseOperation::parse(operation.as_str()),
                Some(operation)
            );
        }
    }
}
