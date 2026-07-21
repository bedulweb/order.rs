use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("config: {0}")]
    Config(String),

    #[error("OCR: {0}")]
    Ocr(String),

    #[error("crypto: {0}")]
    Crypto(String),

    #[error("HTTP: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API {code}: {message}")]
    Api { code: i64, message: String },

    /// BigSeller session expired / not logged in (commonly code 2001).
    #[error("auth expired: {0}")]
    AuthExpired(String),

    #[error("login failed after {attempts} attempts: {last_message}")]
    LoginExhausted {
        attempts: usize,
        last_message: String,
    },

    #[error("not authenticated — run `orders login` first")]
    NotAuthenticated,

    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("image: {0}")]
    Image(#[from] image::ImageError),

    #[error(transparent)]
    Db(#[from] sqlx::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
