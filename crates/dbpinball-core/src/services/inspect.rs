use crate::domain::connection::{DbConnection, DbEngine};
use crate::domain::error::{CoreError, ErrorCode};
use crate::ports::credentials::CredentialProvider;

/// Verify that a connection can be opened.
pub async fn test_connection(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
) -> Result<(), CoreError> {
    match connection.engine {
        DbEngine::Mysql => test_mysql(connection, credentials).await,
        DbEngine::Postgres => test_postgres(connection, credentials).await,
    }
}

/// List databases visible to the connection.
pub async fn list_databases(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
) -> Result<Vec<String>, CoreError> {
    match connection.engine {
        DbEngine::Mysql => list_databases_mysql(connection, credentials).await,
        DbEngine::Postgres => list_databases_postgres(connection, credentials).await,
    }
}

/// List tables in a database.
pub async fn list_tables(
    connection: &DbConnection,
    database: &str,
    credentials: &dyn CredentialProvider,
) -> Result<Vec<String>, CoreError> {
    match connection.engine {
        DbEngine::Mysql => list_tables_mysql(connection, database, credentials).await,
        DbEngine::Postgres => list_tables_postgres(connection, database, credentials).await,
    }
}

fn connection_error(error: sqlx::Error) -> CoreError {
    CoreError::redacted(
        ErrorCode::Connection,
        format!("database connection failed: {error}"),
    )
}

fn decode_mysql_text(row: &sqlx::mysql::MySqlRow, index: usize) -> Result<String, CoreError> {
    // MySQL reports `information_schema` text columns as VARBINARY when the
    // connection negotiates a binary result charset. Decoding the raw bytes
    // works for both VARCHAR and VARBINARY.
    let bytes: Vec<u8> = sqlx::Row::try_get(row, index).map_err(connection_error)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn resolve_database(connection: &DbConnection, requested: &str) -> Result<String, CoreError> {
    if !requested.trim().is_empty() {
        return Ok(requested.to_string());
    }

    if !connection.database.trim().is_empty() {
        return Ok(connection.database.clone());
    }

    Err(CoreError::new(
        ErrorCode::Usage,
        "Database name is required",
    ))
}

async fn test_mysql(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
) -> Result<(), CoreError> {
    let pool = connect_mysql(connection, &connection.database, credentials).await?;
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .map_err(connection_error)?;
    Ok(())
}

async fn test_postgres(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
) -> Result<(), CoreError> {
    let pool = connect_postgres(connection, &connection.database, credentials).await?;
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .map_err(connection_error)?;
    Ok(())
}

async fn list_databases_mysql(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
) -> Result<Vec<String>, CoreError> {
    let fallback = if connection.database.trim().is_empty() {
        "information_schema".to_string()
    } else {
        connection.database.clone()
    };

    let pool = connect_mysql(connection, &fallback, credentials).await?;
    let rows = sqlx::query(
        "SELECT schema_name FROM information_schema.schemata \
         WHERE schema_name NOT IN ('information_schema', 'performance_schema', 'mysql', 'sys') \
         ORDER BY schema_name",
    )
    .fetch_all(&pool)
    .await
    .map_err(connection_error)?;

    rows.iter().map(|row| decode_mysql_text(row, 0)).collect()
}

async fn list_databases_postgres(
    connection: &DbConnection,
    credentials: &dyn CredentialProvider,
) -> Result<Vec<String>, CoreError> {
    let fallback = if connection.database.trim().is_empty() {
        "postgres".to_string()
    } else {
        connection.database.clone()
    };

    let pool = connect_postgres(connection, &fallback, credentials).await?;
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
    )
    .fetch_all(&pool)
    .await
    .map_err(connection_error)?;

    Ok(names)
}

async fn list_tables_mysql(
    connection: &DbConnection,
    database: &str,
    credentials: &dyn CredentialProvider,
) -> Result<Vec<String>, CoreError> {
    let target = resolve_database(connection, database)?;
    let pool = connect_mysql(connection, &target, credentials).await?;

    let rows = sqlx::query(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = ? ORDER BY table_name",
    )
    .bind(&target)
    .fetch_all(&pool)
    .await
    .map_err(connection_error)?;

    rows.iter().map(|row| decode_mysql_text(row, 0)).collect()
}

async fn list_tables_postgres(
    connection: &DbConnection,
    database: &str,
    credentials: &dyn CredentialProvider,
) -> Result<Vec<String>, CoreError> {
    let target = resolve_database(connection, database)?;
    let pool = connect_postgres(connection, &target, credentials).await?;

    sqlx::query_scalar::<_, String>(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = 'public' ORDER BY table_name",
    )
    .fetch_all(&pool)
    .await
    .map_err(connection_error)
}

async fn connect_mysql(
    connection: &DbConnection,
    database: &str,
    credentials: &dyn CredentialProvider,
) -> Result<sqlx::MySqlPool, CoreError> {
    let password = credentials.get(&connection.id)?.expose().to_string();
    let url = format!(
        "mysql://{}:{}@{}:{}/{}",
        connection.username, password, connection.host, connection.port, database
    );

    sqlx::MySqlPool::connect(&url)
        .await
        .map_err(connection_error)
}

async fn connect_postgres(
    connection: &DbConnection,
    database: &str,
    credentials: &dyn CredentialProvider,
) -> Result<sqlx::PgPool, CoreError> {
    let password = credentials.get(&connection.id)?.expose().to_string();
    let url = format!(
        "postgresql://{}:{}@{}:{}/{}",
        connection.username, password, connection.host, connection.port, database
    );

    sqlx::PgPool::connect(&url).await.map_err(connection_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(database: &str) -> DbConnection {
        DbConnection {
            id: "test".into(),
            name: "test".into(),
            engine: DbEngine::Postgres,
            host: "127.0.0.1".into(),
            port: 5432,
            database: database.into(),
            username: "postgres".into(),
            use_ssl: false,
        }
    }

    #[test]
    fn requested_database_takes_precedence() {
        let resolved = resolve_database(&connection("saved"), "requested").unwrap();
        assert_eq!(resolved, "requested");
    }

    #[test]
    fn saved_database_is_used_when_request_is_blank() {
        let resolved = resolve_database(&connection("saved"), "   ").unwrap();
        assert_eq!(resolved, "saved");
    }

    #[test]
    fn missing_database_is_a_usage_error() {
        let error = resolve_database(&connection(""), "").unwrap_err();
        assert_eq!(error.code, ErrorCode::Usage);
    }
}
