#!/usr/bin/env bash
# Install nginx vhost + systemd units for rs.obayito.com (needs sudo).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONF_SRC="$ROOT/deploy/nginx-rs.obayito.com.conf"
UNIT_API="$ROOT/deploy/orders-api.service"
UNIT_WORKER="$ROOT/deploy/orders-worker.service"
DOMAIN=rs.obayito.com

if [[ "${EUID}" -ne 0 ]]; then
  exec sudo -E bash "$0" "$@"
fi

echo "==> nginx site"
install -m 644 "$CONF_SRC" /etc/nginx/sites-available/${DOMAIN}.conf
ln -sfn /etc/nginx/sites-available/${DOMAIN}.conf /etc/nginx/sites-enabled/${DOMAIN}.conf

# HTTP-only first so certbot can challenge (if cert missing, strip SSL server block temporarily)
if [[ ! -f /etc/letsencrypt/live/${DOMAIN}/fullchain.pem ]]; then
  echo "==> temporary HTTP-only vhost (no cert yet)"
  cat >/etc/nginx/sites-available/${DOMAIN}.conf <<EOF
server {
    listen 80;
    listen [::]:80;
    server_name ${DOMAIN};

    location /.well-known/acme-challenge/ {
        root /var/www/certbot;
    }

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }
}
EOF
  mkdir -p /var/www/certbot
  nginx -t
  systemctl reload nginx

  echo "==> certbot"
  certbot --nginx -d "${DOMAIN}" --non-interactive --agree-tos --register-unsafely-without-email \
    || certbot certonly --webroot -w /var/www/certbot -d "${DOMAIN}" --non-interactive --agree-tos --register-unsafely-without-email

  # restore full conf (or leave certbot-edited nginx if certbot --nginx succeeded)
  if [[ -f /etc/letsencrypt/live/${DOMAIN}/fullchain.pem ]]; then
    install -m 644 "$CONF_SRC" /etc/nginx/sites-available/${DOMAIN}.conf
    ln -sfn /etc/nginx/sites-available/${DOMAIN}.conf /etc/nginx/sites-enabled/${DOMAIN}.conf
  fi
fi

nginx -t
systemctl reload nginx

echo "==> systemd"
install -m 644 "$UNIT_API" /etc/systemd/system/orders-api.service
install -m 644 "$UNIT_WORKER" /etc/systemd/system/orders-worker.service
systemctl daemon-reload
systemctl enable --now orders-api.service
# worker optional — enable when ready to pull BigSeller
# systemctl enable --now orders-worker.service

echo "==> done"
systemctl --no-pager status orders-api.service | head -15
curl -sS -m 5 "http://127.0.0.1:8080/health" || true
echo
curl -sS -m 5 "https://${DOMAIN}/health" || curl -sS -m 5 "http://${DOMAIN}/health" || true
echo
