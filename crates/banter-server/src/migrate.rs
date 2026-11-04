//! Versioned, idempotent Postgres schema upgrades.
//!
//! `schema.sql` uses `CREATE TABLE/INDEX IF NOT EXISTS` for greenfield boot.
//! That alone does **not** alter an existing volume when columns or tables are
//! added later. Each entry in [`MIGRATIONS`] runs once per database (tracked in
//! `schema_migrations`) and uses `ALTER … ADD COLUMN IF NOT EXISTS` / new-table
//! creates so long-lived `banter-pg` volumes stay compatible across schema bumps.

use sqlx::{PgPool, Postgres, Transaction};

use crate::error::BanterError;

/// Ordered migrations. Version strings are stable keys — do not rename after ship.
const MIGRATIONS: &[(&str, &str)] = &[(
    "001_profile_customization",
    include_str!("migrations/001_profile_customization.sql"),
)];

/// Transaction-scoped advisory lock so concurrent boots serialize migrate.
const MIGRATE_LOCK_KEY: i64 = 0x4A4E_5452_4D47; // "JNTRMG"

pub async fn apply(pool: &PgPool) -> Result<(), BanterError> {
    let mut tx = pool.begin().await.map_err(db_err)?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(MIGRATE_LOCK_KEY)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    apply_in_tx(&mut tx).await?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

async fn apply_in_tx(tx: &mut Transaction<'_, Postgres>) -> Result<(), BanterError> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version TEXT PRIMARY KEY,
            applied_at BIGINT NOT NULL
        )",
    )
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;

    for (version, sql) in MIGRATIONS {
        let already: Option<String> =
            sqlx::query_scalar("SELECT version FROM schema_migrations WHERE version = $1")
                .bind(version)
                .fetch_optional(&mut **tx)
                .await
                .map_err(db_err)?;
        if already.is_some() {
            continue;
        }

        sqlx::raw_sql(sql).execute(&mut **tx).await.map_err(db_err)?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES ($1, $2)")
            .bind(version)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
    }
    Ok(())
}

fn db_err(err: sqlx::Error) -> BanterError {
    BanterError::Storage(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::MIGRATIONS;

    #[test]
    fn migration_versions_are_unique_and_ordered() {
        assert!(!MIGRATIONS.is_empty());
        let mut seen = std::collections::HashSet::new();
        let mut prev = "";
        for (version, sql) in MIGRATIONS {
            assert!(!version.is_empty(), "empty migration version");
            assert!(!sql.trim().is_empty(), "empty SQL for {version}");
            assert!(
                *version > prev,
                "migrations must be strictly ordered by version string: {prev:?} then {version:?}"
            );
            assert!(seen.insert(*version), "duplicate migration version {version}");
            prev = *version;
        }
    }
}
