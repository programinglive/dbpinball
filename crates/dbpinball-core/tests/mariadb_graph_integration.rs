#![cfg(feature = "integration")]

use dbpinball_core::domain::error::ErrorCode;
use dbpinball_core::safesync::mariadb_graph::{
    discover_mariadb_parents, inspect_mariadb_foreign_keys, inspect_mariadb_table,
    inspect_mariadb_triggers,
};
use dbpinball_core::safesync::spec::QualifiedName;
use sqlx::mysql::MySqlPoolOptions;
use sqlx::MySqlPool;

async fn connect_pair() -> Option<(MySqlPool, MySqlPool, String)> {
    let src_url = std::env::var("DBPINBALL_TEST_MARIADB_URL").ok()?;
    let dst_url = std::env::var("DBPINBALL_TEST_MARIADB_DST_URL")
        .ok()
        .unwrap_or_else(|| src_url.clone());

    let src = MySqlPoolOptions::new()
        .max_connections(4)
        .connect(&src_url)
        .await
        .expect("connect source MariaDB/MySQL");
    let dst = MySqlPoolOptions::new()
        .max_connections(4)
        .connect(&dst_url)
        .await
        .expect("connect destination MariaDB/MySQL");

    let db_name: String = sqlx::query_scalar("SELECT DATABASE()")
        .fetch_one(&src)
        .await
        .expect("query DATABASE()");
    Some((src, dst, db_name))
}

fn q(schema: &str, table: &str) -> QualifiedName {
    QualifiedName {
        schema: schema.to_string(),
        table: table.to_string(),
    }
}

#[tokio::test]
async fn t01_schema_inspection_and_composite_fk_ordering() {
    let Some((src, _dst, db)) = connect_pair().await else {
        eprintln!("skipped: DBPINBALL_TEST_MARIADB_URL not set");
        return;
    };

    sqlx::raw_sql(
        "DROP TABLE IF EXISTS mdb2_items;
         DROP TABLE IF EXISTS mdb2_tenants;
         CREATE TABLE mdb2_tenants (
             org_id BIGINT NOT NULL,
             tenant_id BIGINT NOT NULL,
             code VARCHAR(32) NOT NULL,
             PRIMARY KEY (org_id, tenant_id)
         ) ENGINE=InnoDB;
         CREATE TABLE mdb2_items (
             id BIGINT NOT NULL AUTO_INCREMENT,
             tenant_ref BIGINT NOT NULL,
             org_ref BIGINT NOT NULL,
             PRIMARY KEY (id),
             CONSTRAINT fk_items_tenant
               FOREIGN KEY (org_ref, tenant_ref)
               REFERENCES mdb2_tenants (org_id, tenant_id)
         ) ENGINE=InnoDB;",
    )
    .execute(&src)
    .await
    .expect("setup tables");

    let graph = discover_mariadb_parents(&src, &[q(&db, "mdb2_items")], 4)
        .await
        .expect("discover graph");

    assert_eq!(graph.tables.len(), 2);
    assert_eq!(graph.edges.len(), 1);
    let edge = &graph.edges[0];
    assert_eq!(edge.child_columns, vec!["org_ref", "tenant_ref"]);
    assert_eq!(edge.parent_columns, vec!["org_id", "tenant_id"]);
    assert!(!edge.deferrable);

    let items = graph.table(&q(&db, "mdb2_items")).unwrap();
    assert!(items
        .columns
        .iter()
        .any(|c| c.name == "id" && c.is_identity));
    assert_eq!(items.primary_key, vec!["id"]);

    let fks = inspect_mariadb_foreign_keys(&src, &q(&db, "mdb2_items"))
        .await
        .unwrap();
    assert_eq!(fks.len(), 1);
}

#[tokio::test]
async fn t02_unsupported_storage_engine_and_view_refused() {
    let Some((src, _dst, db)) = connect_pair().await else {
        eprintln!("skipped: DBPINBALL_TEST_MARIADB_URL not set");
        return;
    };

    sqlx::raw_sql(
        "DROP VIEW IF EXISTS mdb2_view;
         DROP TABLE IF EXISTS mdb2_myisam;
         CREATE TABLE mdb2_myisam (id INT PRIMARY KEY) ENGINE=MyISAM;
         CREATE VIEW mdb2_view AS SELECT 1 AS id;",
    )
    .execute(&src)
    .await
    .expect("setup myisam + view");

    let engine_err = inspect_mariadb_table(&src, &q(&db, "mdb2_myisam"))
        .await
        .unwrap_err();
    assert_eq!(engine_err.code, ErrorCode::Unsupported);
    assert_eq!(
        engine_err.detail.as_deref(),
        Some("unsupported_storage_engine")
    );

    let view_err = inspect_mariadb_table(&src, &q(&db, "mdb2_view"))
        .await
        .unwrap_err();
    assert_eq!(view_err.code, ErrorCode::Unsupported);
    assert_eq!(view_err.detail.as_deref(), Some("unsupported_table_type"));
}

#[tokio::test]
async fn t03_trigger_on_table_is_refused() {
    let Some((src, _dst, db)) = connect_pair().await else {
        eprintln!("skipped: DBPINBALL_TEST_MARIADB_URL not set");
        return;
    };

    sqlx::raw_sql(
        "DROP TABLE IF EXISTS mdb2_triggered;
         CREATE TABLE mdb2_triggered (id INT PRIMARY KEY, note VARCHAR(50)) ENGINE=InnoDB;
         CREATE TRIGGER trg_mdb2_ins BEFORE INSERT ON mdb2_triggered
         FOR EACH ROW SET NEW.note = 'triggered';",
    )
    .execute(&src)
    .await
    .expect("setup trigger");

    let triggers = inspect_mariadb_triggers(&src, &q(&db, "mdb2_triggered"))
        .await
        .unwrap();
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0].name, "trg_mdb2_ins");

    let err = inspect_mariadb_table(&src, &q(&db, "mdb2_triggered"))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Unsupported);
    assert_eq!(err.detail.as_deref(), Some("triggers_not_supported"));
}

#[tokio::test]
async fn t04_self_reference_and_fk_cycles_are_refused() {
    let Some((src, _dst, db)) = connect_pair().await else {
        eprintln!("skipped: DBPINBALL_TEST_MARIADB_URL not set");
        return;
    };

    sqlx::raw_sql(
        "DROP TABLE IF EXISTS mdb2_self;
         CREATE TABLE mdb2_self (
             id INT PRIMARY KEY,
             parent_id INT NULL,
             CONSTRAINT fk_self FOREIGN KEY (parent_id) REFERENCES mdb2_self (id)
         ) ENGINE=InnoDB;",
    )
    .execute(&src)
    .await
    .expect("setup self ref");

    let self_err = discover_mariadb_parents(&src, &[q(&db, "mdb2_self")], 4)
        .await
        .unwrap_err();
    assert_eq!(self_err.code, ErrorCode::Unsupported);
    assert_eq!(self_err.detail.as_deref(), Some("cyclic_fk_not_supported"));

    sqlx::raw_sql(
        "SET FOREIGN_KEY_CHECKS=0;
         DROP TABLE IF EXISTS mdb2_cyc_a;
         DROP TABLE IF EXISTS mdb2_cyc_b;
         CREATE TABLE mdb2_cyc_a (id INT PRIMARY KEY, b_id INT) ENGINE=InnoDB;
         CREATE TABLE mdb2_cyc_b (id INT PRIMARY KEY, a_id INT) ENGINE=InnoDB;
         ALTER TABLE mdb2_cyc_a ADD CONSTRAINT fk_ab FOREIGN KEY (b_id) REFERENCES mdb2_cyc_b (id);
         ALTER TABLE mdb2_cyc_b ADD CONSTRAINT fk_ba FOREIGN KEY (a_id) REFERENCES mdb2_cyc_a (id);
         SET FOREIGN_KEY_CHECKS=1;",
    )
    .execute(&src)
    .await
    .expect("setup cycle");

    let cyc_err = discover_mariadb_parents(&src, &[q(&db, "mdb2_cyc_a")], 4)
        .await
        .unwrap_err();
    assert_eq!(cyc_err.code, ErrorCode::Unsupported);
    assert_eq!(cyc_err.detail.as_deref(), Some("cyclic_fk_not_supported"));
}

#[tokio::test]
async fn t05_missing_table_and_schema_drift_across_two_instances() {
    let Some((src, dst, db)) = connect_pair().await else {
        eprintln!("skipped: DBPINBALL_TEST_MARIADB_URL not set");
        return;
    };

    let missing = inspect_mariadb_table(&src, &q(&db, "mdb2_does_not_exist"))
        .await
        .unwrap_err();
    assert_eq!(missing.code, ErrorCode::NotFound);

    sqlx::raw_sql(
        "DROP TABLE IF EXISTS mdb2_drift;
         CREATE TABLE mdb2_drift (id BIGINT PRIMARY KEY, amount DECIMAL(12,2) NOT NULL) ENGINE=InnoDB;",
    )
    .execute(&src)
    .await
    .expect("setup src drift");

    let dst_db: String = sqlx::query_scalar("SELECT DATABASE()")
        .fetch_one(&dst)
        .await
        .unwrap();
    sqlx::raw_sql(
        "DROP TABLE IF EXISTS mdb2_drift;
         CREATE TABLE mdb2_drift (id BIGINT PRIMARY KEY, amount DECIMAL(12,2) NOT NULL) ENGINE=InnoDB;",
    )
    .execute(&dst)
    .await
    .expect("setup dst drift same");

    let src_info = inspect_mariadb_table(&src, &q(&db, "mdb2_drift"))
        .await
        .unwrap();
    let dst_same = inspect_mariadb_table(&dst, &q(&dst_db, "mdb2_drift"))
        .await
        .unwrap();
    assert_eq!(src_info.schema_hash, dst_same.schema_hash);

    sqlx::raw_sql("ALTER TABLE mdb2_drift MODIFY COLUMN amount VARCHAR(32) NOT NULL;")
        .execute(&dst)
        .await
        .expect("alter dst drift");

    let dst_drifted = inspect_mariadb_table(&dst, &q(&dst_db, "mdb2_drift"))
        .await
        .unwrap();
    assert_ne!(src_info.schema_hash, dst_drifted.schema_hash);
}

#[tokio::test]
async fn t06_insufficient_privileges_fail_closed() {
    let Some((src, _dst, db)) = connect_pair().await else {
        eprintln!("skipped: DBPINBALL_TEST_MARIADB_URL not set");
        return;
    };

    sqlx::raw_sql(
        "DROP TABLE IF EXISTS mdb2_secret_shape;
         CREATE TABLE mdb2_secret_shape (id INT PRIMARY KEY) ENGINE=InnoDB;",
    )
    .execute(&src)
    .await
    .expect("setup secret shape");

    let unpriv_url = std::env::var("DBPINBALL_TEST_MARIADB_UNPRIV_URL").ok();
    if let Some(unpriv_url) = unpriv_url {
        let unpriv = MySqlPoolOptions::new()
            .max_connections(2)
            .connect(&unpriv_url)
            .await
            .expect("connect unpriv user");
        let err = inspect_mariadb_table(&unpriv, &q(&db, "mdb2_secret_shape"))
            .await
            .unwrap_err();
        assert!(matches!(
            err.code,
            ErrorCode::NotFound | ErrorCode::PolicyRefusal | ErrorCode::Connection
        ));
    }
}
