# Credentials and Infisical

Never commit real credentials. Local `.env` and `.session.json` are gitignored.

## Preferred: Infisical

1. Create or open an Infisical project (plan must allow it).
2. From this repo:

```bash
infisical init
# select org + project; writes .infisical.json (workspace id only -- safe to commit)
```

3. Put secrets in Infisical env `dev` (and `prod` when ready).

| Secret | Required | Notes |
|--------|----------|--------|
| `BS_ACCOUNT` | yes | BigSeller login email/phone |
| `BS_PASSWORD` | yes | BigSeller password |
| `DATABASE_URL` | yes | Neon `postgresql://...?sslmode=require` |
| `API_TOKEN` | yes | Shared bearer for internal HTTP API |
| `BS_ACCOUNT_CODE` | no | Tenant slug, default `default` |
| `API_BIND` | no | default `0.0.0.0:8080` |
| `SYNC_NEW_INTERVAL_SECS` | no | default `60` |
| `CANCEL_HOUR_LOCAL` | no | default `17` |
| `CANCEL_MINUTE_LOCAL` | no | default `0` |
| `AUTO_RELOGIN` | no | default `true` |
| `WA_WEBHOOK_URL` | no | optional notify webhook |
| `WA_WEBHOOK_TOKEN` | no | optional bearer for webhook |
| `BS_SESSION_PATH` | no | default `.session.json` |
| `BS_MODEL_PATH` | no | default `models/common_old.onnx` |
| `BS_CHARSET_PATH` | no | default `models/charset.json` |
| `BS_BASE_URL` | no | default `https://www.bigseller.com` |

4. Upload from a local gitignored `.env` (one-time bootstrap):

```bash
./scripts/push-secrets-infisical.sh
```

5. Run:

```bash
infisical run --env=dev -- ./target/release/orders doctor
infisical run --env=dev -- ./target/release/orders worker
infisical run --env=dev -- ./target/release/orders serve
```

Machine identity (Universal Auth) used by `linux-devkit` must be granted access to the Infisical project, or use interactive `infisical login` as a user.

## Local fallback

```bash
cp .env.example .env
# edit values locally -- file stays gitignored
./target/release/orders doctor
```

## Rotate

If a Neon password, API token, or BigSeller password was ever pasted into chat or a ticket, rotate it in the provider dashboard and update Infisical.
