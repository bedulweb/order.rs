# Multi-stage: web UI (vite) + Rust release + model OCR ddddocr → runtime.
# Deploy: git push dokku main (dokku dockerfile builder).
#
# Build & runtime pakai Ubuntu 24.04 (glibc 2.39): prebuilt onnxruntime dari
# ort memakai simbol __isoc23_* yang hanya ada di glibc >= 2.38.

# --- Stage 1: build web UI ---
FROM node:22-alpine AS web
WORKDIR /app/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# --- Stage 2: Rust release build ---
FROM ubuntu:24.04 AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential curl ca-certificates pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.97.1
ENV PATH="/root/.cargo/bin:${PATH}"
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY examples/ examples/
COPY tests/ tests/
RUN --mount=type=cache,target=/app/target cargo build --release --locked

# --- Stage 3: ambil model captcha ddddocr (tidak di-track git) ---
FROM python:3.12-slim AS model
RUN pip install --no-cache-dir ddddocr
RUN python -c "import ddddocr, os, shutil; shutil.copy(os.path.join(os.path.dirname(ddddocr.__file__), 'common_old.onnx'), '/common_old.onnx')"

# --- Stage 4: runtime ---
FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates tzdata fonts-dejavu-core libgcc-s1 libstdc++6 \
    && rm -rf /var/lib/apt/lists/*
ENV TZ=Asia/Jakarta
WORKDIR /app
COPY --from=build /app/target/release/orders /app/orders
COPY --from=web /app/web/dist /app/web/dist
COPY --from=model /common_old.onnx /app/models/common_old.onnx
COPY models/charset.json /app/models/charset.json
ENV BS_MODEL_PATH=/app/models/common_old.onnx \
    BS_CHARSET_PATH=/app/models/charset.json \
    BS_SESSION_PATH=/data/.session.json \
    WEB_DIST=/app/web/dist \
    RUST_LOG=info
EXPOSE 8080
# Worker + API dalam satu proses (worker background, serve foreground di $PORT dokku).
CMD ["sh", "-c", "mkdir -p /data && ./orders worker & exec ./orders serve --bind 0.0.0.0:${PORT:-8080}"]
