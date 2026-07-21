#!/usr/bin/env bash
# Upload gitignored .env into Infisical (current directory project).
# Requires: infisical CLI, linked .infisical.json (infisical init), login.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

ENV_FILE="${1:-.env}"
INF_ENV="${INFISICAL_ENV:-dev}"

if [[ ! -f "$ENV_FILE" ]]; then
  echo "missing $ENV_FILE (create from .env.example or export from Infisical first)"
  exit 1
fi

if [[ ! -f .infisical.json ]]; then
  echo "missing .infisical.json -- run: infisical init"
  exit 1
fi

if ! command -v infisical >/dev/null 2>&1; then
  echo "infisical CLI not found"
  exit 1
fi

echo "pushing secrets from $ENV_FILE -> Infisical env=$INF_ENV"
# --file accepts dotenv; does not print values
infisical secrets set --env="$INF_ENV" --file="$ENV_FILE"
echo "done. verify with: infisical secrets --env=$INF_ENV"
