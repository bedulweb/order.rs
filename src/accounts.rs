//! BigSeller tenant rows in Postgres (`bs_accounts` + `bs_sessions`).

use crate::error::{Error, Result};
use crate::session::SessionData;
use serde_json::json;
use sqlx::{PgPool, Row};

#[derive(Debug, Clone)]
pub struct Account {
    pub id: i64,
    pub code: String,
    pub login_account: String,
    pub display_name: Option<String>,
}

/// Ensure a tenant row exists for this login / code. Idempotent.
pub async fn ensure_account(
    pool: &PgPool,
    code: &str,
    login_account: &str,
    display_name: Option<&str>,
) -> Result<Account> {
    let code = code.trim();
    let login = login_account.trim();
    if code.is_empty() || login.is_empty() {
        return Err(Error::Config(
            "account code and login_account are required".into(),
        ));
    }

    // Prefer match by code; fall back to login_account (upgrade path).
    let existing = sqlx::query(
        r#"
        SELECT id, code, login_account, display_name
        FROM bs_accounts
        WHERE code = $1 OR login_account = $2
        ORDER BY CASE WHEN code = $1 THEN 0 ELSE 1 END
        LIMIT 1
        "#,
    )
    .bind(code)
    .bind(login)
    .fetch_optional(pool)
    .await
    .map_err(|e| Error::Db(e.to_string()))?;

    if let Some(row) = existing {
        let id: i64 = row.get("id");
        sqlx::query(
            r#"
            UPDATE bs_accounts
            SET code = $2,
                login_account = $3,
                display_name = COALESCE($4, display_name),
                updated_at = now()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(code)
        .bind(login)
        .bind(display_name)
        .execute(pool)
        .await
        .map_err(|e| Error::Db(e.to_string()))?;

        return Ok(Account {
            id,
            code: code.to_string(),
            login_account: login.to_string(),
            display_name: display_name.map(|s| s.to_string()).or_else(|| {
                row.try_get::<Option<String>, _>("display_name")
                    .ok()
                    .flatten()
            }),
        });
    }

    let row = sqlx::query(
        r#"
        INSERT INTO bs_accounts (login_account, code, display_name)
        VALUES ($1, $2, $3)
        RETURNING id, code, login_account, display_name
        "#,
    )
    .bind(login)
    .bind(code)
    .bind(display_name)
    .fetch_one(pool)
    .await
    .map_err(|e| Error::Db(e.to_string()))?;

    Ok(Account {
        id: row.get("id"),
        code: row.get("code"),
        login_account: row.get("login_account"),
        display_name: row.get("display_name"),
    })
}

pub async fn save_session_row(pool: &PgPool, account_id: i64, session: &SessionData) -> Result<()> {
    let cookies = serde_json::to_value(&session.cookies).unwrap_or_else(|_| json!({}));
    sqlx::query(
        r#"
        INSERT INTO bs_sessions (account_id, cookies, access_token, is_valid, last_login_at, last_check_at, updated_at)
        VALUES ($1, $2, $3, true, now(), now(), now())
        ON CONFLICT (account_id) DO UPDATE SET
            cookies = EXCLUDED.cookies,
            access_token = EXCLUDED.access_token,
            is_valid = true,
            last_login_at = now(),
            last_check_at = now(),
            updated_at = now()
        "#,
    )
    .bind(account_id)
    .bind(cookies)
    .bind(&session.access_token)
    .execute(pool)
    .await
    .map_err(|e| Error::Db(e.to_string()))?;
    Ok(())
}

pub async fn mark_session_checked(pool: &PgPool, account_id: i64, valid: bool) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE bs_sessions
        SET is_valid = $2, last_check_at = now(), updated_at = now()
        WHERE account_id = $1
        "#,
    )
    .bind(account_id)
    .bind(valid)
    .execute(pool)
    .await
    .map_err(|e| Error::Db(e.to_string()))?;
    Ok(())
}

pub async fn get_account_by_code(pool: &PgPool, code: &str) -> Result<Option<Account>> {
    let row = sqlx::query(
        r#"
        SELECT id, code, login_account, display_name
        FROM bs_accounts WHERE code = $1
        "#,
    )
    .bind(code)
    .fetch_optional(pool)
    .await
    .map_err(|e| Error::Db(e.to_string()))?;

    Ok(row.map(|r| Account {
        id: r.get("id"),
        code: r.get("code"),
        login_account: r.get("login_account"),
        display_name: r.get("display_name"),
    }))
}

pub async fn count_orders(pool: &PgPool, account_id: Option<i64>) -> Result<i64> {
    let row = if let Some(aid) = account_id {
        sqlx::query(r#"SELECT COUNT(*)::bigint AS c FROM orders WHERE account_id = $1"#)
            .bind(aid)
            .fetch_one(pool)
            .await
    } else {
        sqlx::query(r#"SELECT COUNT(*)::bigint AS c FROM orders"#)
            .fetch_one(pool)
            .await
    }
    .map_err(|e| Error::Db(e.to_string()))?;
    Ok(row.get("c"))
}

pub async fn latest_sync_summary(pool: &PgPool) -> Result<serde_json::Value> {
    let rows = sqlx::query(
        r#"
        SELECT id, kind, status, started_at, finished_at,
               pages_fetched, rows_upserted, error_text
        FROM sync_runs
        ORDER BY id DESC
        LIMIT 10
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| Error::Db(e.to_string()))?;

    let list: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.get::<i64, _>("id"),
                "kind": r.get::<String, _>("kind"),
                "status": r.get::<String, _>("status"),
                "startedAt": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("started_at"),
                "finishedAt": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("finished_at"),
                "pagesFetched": r.get::<i32, _>("pages_fetched"),
                "rowsUpserted": r.get::<i32, _>("rows_upserted"),
                "errorText": r.get::<Option<String>, _>("error_text"),
            })
        })
        .collect();

    Ok(json!({ "recentRuns": list }))
}
