//! CLI: login | list | counts | sync | worker | serve | doctor | ocr | status

use anyhow::Context;
use clap::{Parser, Subcommand};
use orders::{
    accounts,
    api::{self, ApiState},
    db, login, sync, CaptchaOcr, Config, OrderListQuery, OrderSummary, OrdersApi, SessionData,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "orders",
    about = "BigSeller sync worker + internal order API",
    version
)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Login with BS_ACCOUNT / BS_PASSWORD (OCR captcha, save session)
    Login {
        #[arg(long, env = "BS_LOGIN_ATTEMPTS")]
        attempts: Option<usize>,
    },

    /// List orders from BigSeller (requires prior login)
    List {
        #[arg(long, default_value = "new")]
        status: String,
        #[arg(long, default_value_t = 1)]
        page: u32,
        #[arg(long, default_value_t = 50)]
        page_size: u32,
        #[arg(long)]
        json: bool,
    },

    /// Order status counts from BigSeller
    Counts {
        #[arg(long)]
        json: bool,
    },

    /// One-shot: pull BigSeller → Postgres
    Sync {
        /// Status bucket: new | cancel | all | <raw status>
        #[arg(long, default_value = "new")]
        status: String,
    },

    /// Long-running: poll new + evening cancel + outbox + auto re-login
    Worker,

    /// HTTP API (lookup / events / cancel report / sync status)
    Serve {
        #[arg(long, env = "API_BIND")]
        bind: Option<String>,
    },

    /// Check env, DB, session, OCR model
    Doctor,

    /// OCR a local captcha image (debug)
    Ocr { path: PathBuf },

    /// Show session file status
    Status,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let _ = dotenvy::from_path(std::path::Path::new(&manifest).join(".env"));
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                EnvFilter::new("info,ort=error,ort::logging=error,ort_sys=error")
            }),
        )
        .init();

    let cli = Cli::parse();
    let mut cfg = Config::from_env().context("load config")?;

    match cli.cmd {
        Command::Login { attempts } => {
            if let Some(a) = attempts {
                cfg.login_attempts = a;
            }
            cfg.validate_paths()?;
            let ocr = CaptchaOcr::load(&cfg.model_path, &cfg.charset_path, cfg.ocr_threads)?;
            ocr.warmup()?;
            let result = login(&cfg, &ocr).await?;
            println!(
                "OK login in {} attempt(s), captcha={}, session={}",
                result.attempts,
                result.captcha_used,
                cfg.session_path.display()
            );
            if result.session.has_auth() {
                println!(
                    "auth: muc_token present ({} cookies)",
                    result.session.cookies.len()
                );
            } else {
                println!("auth: warning — no muc_token in session; API calls may fail");
            }
            // Persist tenant + session to DB when DATABASE_URL is set.
            if let Ok(db_url) = cfg.require_database_url() {
                match db::connect(db_url).await {
                    Ok(pool) => match sync::bootstrap_account(&pool, &cfg).await {
                        Ok(acct) => {
                            let _ =
                                accounts::save_session_row(&pool, acct.id, &result.session).await;
                            println!(
                                "account: id={} code={} login={}",
                                acct.id, acct.code, acct.login_account
                            );
                        }
                        Err(e) => eprintln!("warn: could not upsert bs_accounts: {e}"),
                    },
                    Err(e) => eprintln!("warn: db connect failed: {e}"),
                }
            }
        }

        Command::List {
            status,
            page,
            page_size,
            json,
        } => {
            let session = SessionData::load(&cfg.session_path)?;
            let api = OrdersApi::new(&cfg.base_url, &session)?;
            let q = OrderListQuery {
                status,
                page_no: page,
                page_size,
                ..Default::default()
            };
            let page = api.page_list(&q).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&page.raw)?);
            } else {
                println!("total={} rows={}", page.total, page.rows.len());
                for (i, row) in page.rows.iter().enumerate() {
                    let s = OrderSummary::from_row(row);
                    println!(
                        "{:>3}. {} | {} | {} | {} | {}",
                        i + 1,
                        s.platform_order_id.as_deref().unwrap_or("-"),
                        s.platform.as_deref().unwrap_or("-"),
                        s.shop_name.as_deref().unwrap_or("-"),
                        s.buyer.as_deref().unwrap_or("-"),
                        s.amount.as_deref().unwrap_or("-"),
                    );
                }
            }
        }

        Command::Counts { json } => {
            let session = SessionData::load(&cfg.session_path)?;
            let api = OrdersApi::new(&cfg.base_url, &session)?;
            let data = api.status_counts().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&data)?);
            } else if let Some(obj) = data.as_object() {
                let mut keys: Vec<_> = obj.keys().collect();
                keys.sort();
                for k in keys {
                    println!("{k}: {}", obj[k]);
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&data)?);
            }
        }

        Command::Sync { status } => {
            let db_url = cfg.require_database_url()?;
            let pool = db::connect(db_url).await?;
            db::ping(&pool).await?;
            let account = sync::bootstrap_account(&pool, &cfg).await?;
            let ctx = sync::SyncContext {
                account_id: Some(account.id),
                account_code: Some(account.code.clone()),
            };
            println!("account id={} code={}", account.id, account.code);

            let session = SessionData::load(&cfg.session_path)?;
            let api = OrdersApi::new(&cfg.base_url, &session)?;

            match status.as_str() {
                "all" => {
                    let s = sync::sync_new_orders(&pool, &api, &ctx).await?;
                    println!(
                        "new: pages={} upserted={} created={}",
                        s.pages, s.upserted, s.created
                    );
                    let stats = sync::sync_cancel_related(&pool, &api, &ctx).await?;
                    for s in stats {
                        println!(
                            "{}: pages={} upserted={} created={}",
                            s.kind, s.pages, s.upserted, s.created
                        );
                    }
                }
                "cancel" | "canceled" | "in-cancel" => {
                    let stats = sync::sync_cancel_related(&pool, &api, &ctx).await?;
                    for s in stats {
                        println!(
                            "{}: pages={} upserted={} created={}",
                            s.kind, s.pages, s.upserted, s.created
                        );
                    }
                }
                other => {
                    let s = sync::sync_status_bucket(&pool, &api, other, 50, 80, &ctx).await?;
                    println!(
                        "{}: pages={} upserted={} created={} state_changed={}",
                        s.kind, s.pages, s.upserted, s.created, s.state_changed
                    );
                }
            }
        }

        Command::Worker => {
            let db_url = cfg.require_database_url()?;
            let pool = db::connect(db_url).await?;
            db::ping(&pool).await?;
            let wcfg = sync::WorkerConfig {
                new_interval_secs: cfg.sync_new_interval_secs,
                cancel_hour_local: cfg.cancel_hour_local,
                cancel_minute_local: cfg.cancel_minute_local,
                wa_webhook_url: cfg.wa_webhook_url.clone(),
                wa_webhook_token: cfg.wa_webhook_token.clone(),
                auto_relogin: cfg.auto_relogin,
            };
            println!(
                "worker: new every {}s, cancel {:02}:{:02} local, auto_relogin={}, webhook={}",
                wcfg.new_interval_secs,
                wcfg.cancel_hour_local,
                wcfg.cancel_minute_local,
                wcfg.auto_relogin,
                wcfg.wa_webhook_url.is_some()
            );
            sync::run_worker(pool, cfg, wcfg).await?;
        }

        Command::Serve { bind } => {
            let db_url = cfg.require_database_url()?;
            let pool = db::connect(db_url).await?;
            db::ping(&pool).await?;
            let bind_s = bind.unwrap_or(cfg.api_bind.clone());
            let addr: SocketAddr = bind_s
                .parse()
                .with_context(|| format!("invalid API_BIND / --bind: {bind_s}"))?;
            if cfg.api_token.is_none() {
                tracing::warn!("API_TOKEN not set — API is open (dev only)");
            }
            let state = ApiState {
                pool,
                api_token: cfg.api_token.clone(),
            };
            api::serve(state, addr).await?;
        }

        Command::Doctor => {
            run_doctor(&cfg).await?;
        }

        Command::Ocr { path } => {
            cfg.validate_paths()?;
            let ocr = CaptchaOcr::load(&cfg.model_path, &cfg.charset_path, cfg.ocr_threads)?;
            ocr.warmup()?;
            let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
            let t0 = std::time::Instant::now();
            let text = ocr.classify_bytes(&bytes)?;
            println!("{text}\t{:.0?}", t0.elapsed());
        }

        Command::Status => match SessionData::load(&cfg.session_path) {
            Ok(s) => {
                println!("session: {}", cfg.session_path.display());
                println!("account: {:?}", s.account);
                println!("saved_at: {:?}", s.saved_at);
                println!("cookies: {}", s.cookies.len());
                println!(
                    "muc_token: {}",
                    if s.cookies.contains_key("muc_token") {
                        "yes"
                    } else {
                        "no"
                    }
                );
                println!(
                    "database: {}",
                    if cfg.database_url.is_some() {
                        "set"
                    } else {
                        "missing"
                    }
                );
                println!(
                    "account_code: {}",
                    cfg.account_code.as_deref().unwrap_or("default")
                );
            }
            Err(e) => {
                println!("no session ({}): {}", cfg.session_path.display(), e);
                std::process::exit(1);
            }
        },
    }

    Ok(())
}

async fn run_doctor(cfg: &Config) -> anyhow::Result<()> {
    let mut ok = true;
    println!("== orders doctor ==");

    // Env
    let checks = [
        (
            "BS_ACCOUNT",
            cfg.account.as_ref().map(|s| !s.is_empty()).unwrap_or(false),
        ),
        (
            "BS_PASSWORD",
            cfg.password
                .as_ref()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
        ),
        (
            "DATABASE_URL",
            cfg.database_url
                .as_ref()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
        ),
        (
            "API_TOKEN",
            cfg.api_token
                .as_ref()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
        ),
    ];
    for (name, good) in checks {
        println!("  env {name}: {}", if good { "ok" } else { "MISSING" });
        if !good && name != "API_TOKEN" {
            // API_TOKEN warning only
            if name != "API_TOKEN" {
                ok = false;
            }
        }
        if !good && name == "API_TOKEN" {
            println!("         (warn: API open without token)");
        }
        if !good && (name == "BS_ACCOUNT" || name == "BS_PASSWORD" || name == "DATABASE_URL") {
            ok = false;
        }
    }
    println!(
        "  BS_ACCOUNT_CODE: {}",
        cfg.account_code.as_deref().unwrap_or("default")
    );
    println!("  AUTO_RELOGIN: {}", cfg.auto_relogin);
    println!("  SYNC_NEW_INTERVAL_SECS: {}", cfg.sync_new_interval_secs);
    println!(
        "  CANCEL local: {:02}:{:02}",
        cfg.cancel_hour_local, cfg.cancel_minute_local
    );

    // Paths
    match cfg.validate_paths() {
        Ok(()) => println!("  OCR model: ok ({})", cfg.model_path.display()),
        Err(e) => {
            println!("  OCR model: FAIL ({e})");
            ok = false;
        }
    }
    println!(
        "  charset: {}",
        if cfg.charset_path.is_file() {
            "ok"
        } else {
            "MISSING"
        }
    );

    // Session
    match SessionData::load(&cfg.session_path) {
        Ok(s) => {
            println!(
                "  session file: ok (muc_token={}, cookies={})",
                s.has_auth(),
                s.cookies.len()
            );
            if s.has_auth() {
                match OrdersApi::new(&cfg.base_url, &s) {
                    Ok(api) => match api.is_login().await {
                        Ok(true) => println!("  BigSeller isLogin: true"),
                        Ok(false) => {
                            println!("  BigSeller isLogin: false (need login)");
                            ok = false;
                        }
                        Err(e) => {
                            println!("  BigSeller isLogin: error ({e})");
                            ok = false;
                        }
                    },
                    Err(e) => println!("  BigSeller client: error ({e})"),
                }
            }
        }
        Err(_) => println!("  session file: missing (run `orders login`)"),
    }

    // DB
    if let Some(url) = cfg.database_url.as_deref() {
        match db::connect(url).await {
            Ok(pool) => match db::ping(&pool).await {
                Ok(()) => {
                    println!("  postgres: ok");
                    let n = accounts::count_orders(&pool, None).await.unwrap_or(-1);
                    println!("  orders cached: {n}");
                    if let Ok(summary) = accounts::latest_sync_summary(&pool).await {
                        if let Some(arr) = summary.get("recentRuns").and_then(|v| v.as_array()) {
                            println!("  recent sync_runs: {}", arr.len());
                            if let Some(last) = arr.first() {
                                println!(
                                    "    last: kind={} status={} rows={}",
                                    last.get("kind").and_then(|v| v.as_str()).unwrap_or("?"),
                                    last.get("status").and_then(|v| v.as_str()).unwrap_or("?"),
                                    last.get("rowsUpserted")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0)
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("  postgres ping: FAIL ({e})");
                    ok = false;
                }
            },
            Err(e) => {
                println!("  postgres connect: FAIL ({e})");
                ok = false;
            }
        }
    }

    if ok {
        println!("== doctor: OK ==");
        Ok(())
    } else {
        println!("== doctor: ISSUES (see above) ==");
        std::process::exit(1);
    }
}
