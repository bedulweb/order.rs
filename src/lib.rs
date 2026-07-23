//! BigSeller orders client + Postgres cache + internal HTTP API.
//!
//! ## Modules
//! - [`ocr`] — DIY ddddocr (`common_old.onnx` + CTC) captcha for BigSeller login
//! - [`screen_ocr`] — [`ocrs`] scene OCR for Shopee/WA screenshot order ids
//! - [`auth`] — captcha login + AES password
//! - [`orders`] — authenticated order list APIs
//! - [`session`] — cookie persistence
//! - [`accounts`] — multi-tenant `bs_accounts` rows
//! - [`sync`] — pull pageList → Neon
//! - [`api`] — read-only HTTP for consumers
//! - [`store`] — SQL upserts / lookups

pub mod accounts;
pub mod api;
pub mod auth;
pub mod batch;
pub mod batch_pdf;
pub mod cancel_notify;
pub mod carrier_day_list;
pub mod catalog;
pub mod client;
pub mod config;
pub mod crypto;
pub mod daily_infographic;
pub mod daily_report;
pub mod db;
pub mod error;
pub mod instant_notify;
pub mod map;
pub mod notify;
pub mod ocr;
pub mod orders;
pub mod product_names;
pub mod screen_ocr;
pub mod session;
pub mod store;
pub mod sync;
pub mod wazapin;

pub use auth::{login, LoginResult};
pub use config::Config;
pub use error::{Error, Result};
pub use ocr::CaptchaOcr;
pub use orders::{OrderListQuery, OrderPage, OrderSummary, OrdersApi};
pub use screen_ocr::{
    extract_order_id_from_bytes, extract_order_id_from_image, extract_order_ids,
    extract_order_ids_from_bytes, OrderIdHit, OrderIdKind,
};
pub use session::SessionData;
