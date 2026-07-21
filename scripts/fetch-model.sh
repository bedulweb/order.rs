#!/usr/bin/env bash
# Copy Python ddddocr default model into models/common_old.onnx
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/models/common_old.onnx"

if [[ -f "$DEST" ]]; then
  echo "already present: $DEST"
  exit 0
fi

# Prefer an existing local ddddocr install / venv
CANDIDATES=(
  "${DDDDOCR_MODEL:-}"
  /tmp/ocrvenv/lib/python*/site-packages/ddddocr/common_old.onnx
  "$HOME"/.local/lib/python*/site-packages/ddddocr/common_old.onnx
  /usr/local/lib/python*/site-packages/ddddocr/common_old.onnx
)

for pattern in "${CANDIDATES[@]}"; do
  [[ -z "$pattern" ]] && continue
  # shellcheck disable=SC2086
  for f in $pattern; do
    if [[ -f "$f" ]]; then
      cp -v "$f" "$DEST"
      echo "OK -> $DEST"
      exit 0
    fi
  done
done

if command -v python3 >/dev/null 2>&1; then
  py_path="$(python3 - <<'PY'
try:
    import ddddocr, os
    p = os.path.join(os.path.dirname(ddddocr.__file__), "common_old.onnx")
    print(p if os.path.isfile(p) else "")
except Exception:
    print("")
PY
)"
  if [[ -n "$py_path" && -f "$py_path" ]]; then
    cp -v "$py_path" "$DEST"
    echo "OK -> $DEST"
    exit 0
  fi
fi

cat <<EOF
Could not find common_old.onnx.

Install Python ddddocr once, then re-run:
  pip install ddddocr
  ./scripts/fetch-model.sh

Or set DDDDOCR_MODEL=/path/to/common_old.onnx
EOF
exit 1
