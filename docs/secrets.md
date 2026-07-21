# Credentials

Store secrets in a local **`.env`** file (gitignored). Never commit it.

## Setup

```bash
cp .env.example .env
# edit values
```

Required:

| Name | Purpose |
|------|---------|
| `BS_ACCOUNT` | BigSeller login |
| `BS_PASSWORD` | BigSeller password |
| `DATABASE_URL` | Neon Postgres URL |
| `API_TOKEN` | Bearer token for HTTP API |

Optional: `BS_ACCOUNT_CODE` (default `default`), `API_BIND`, `SYNC_NEW_INTERVAL_SECS`, `CANCEL_HOUR_LOCAL`, `CANCEL_MINUTE_LOCAL`, `AUTO_RELOGIN`, `WA_WEBHOOK_*`, model/session paths. See `.env.example`.

The binary loads `.env` from the current working directory and from the crate root (`CARGO_MANIFEST_DIR`).

```bash
./target/release/orders doctor
./target/release/orders login
./target/release/orders worker
./target/release/orders serve
```

## Do not commit

- `.env`
- `.session.json` (cookies after login)
- `models/*.onnx`

All of these are listed in `.gitignore`.

## Rotate

If a password or Neon URL was exposed (chat, screenshot, etc.), rotate it at the provider and update `.env`.
