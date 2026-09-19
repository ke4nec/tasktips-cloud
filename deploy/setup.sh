#!/bin/sh
# 配置与密钥管理：本地试用直接用 env.example 预定义默认值 + quickstart.sh 一键启动；
# 本脚本负责补齐缺失项（JWT 私钥）与生产硬化（--force 换强随机密钥、--domain 写公网地址）。
#
#   bash deploy/quickstart.sh                            # 本地试用（推荐）
#   bash deploy/setup.sh [--domain https://example.com] [--admin-origin URL] [--force] [--quiet]
#
# Generates deploy/.env secrets (database/object-store passwords, cursor
# secret) and the Ed25519 JWT private key, then prints the generated values
# ONCE to this terminal. Afterwards the whole stack starts with:
#
#   docker compose --env-file deploy/.env -f deploy/compose.yaml pull
#   docker compose --env-file deploy/.env -f deploy/compose.yaml up -d
#
# Idempotent: existing .env entries and the JWT key are never overwritten
# (pass --force to regenerate; warning: postgres/RustFS volumes keep the old
# credentials, so --force on a live stack locks it out unless volumes are
# wiped too). Generated passwords use letters and digits only so they stay
# valid inside the database URL that deploy/compose.yaml assembles.
#
# Secrets are stored in deploy/.env (git-ignored) and printed to this
# terminal only. They are intentionally never written to container logs.
set -eu

DEPLOY_DIR=$(dirname "$0")
cd "$DEPLOY_DIR"

DOMAIN=""
ADMIN_ORIGIN=""
FORCE=0
QUIET=0

usage() {
  echo "用法: bash deploy/setup.sh [--domain https://example.com] [--admin-origin URL] [--force] [--quiet]"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --domain)
      DOMAIN="${2:?--domain requires a URL}"; shift 2;;
    --admin-origin)
      ADMIN_ORIGIN="${2:?--admin-origin requires a URL}"; shift 2;;
    --force)
      FORCE=1; shift;;
    --quiet)
      QUIET=1; shift;;
    -h|--help)
      usage; exit 0;;
    *)
      echo "未知参数: $1" >&2; usage >&2; exit 1;;
  esac
done

command -v openssl >/dev/null 2>&1 || {
  echo "需要 openssl，请先安装后再运行。" >&2; exit 1;
}

rand_alnum() {
  tr -dc 'A-Za-z0-9' </dev/urandom | head -c "$1"
}

GENERATED=""

# Append KEY=VALUE only when KEY is absent (commented placeholders do not count).
ensure_env() {
  key="$1"
  value="$2"
  if [ "$FORCE" -eq 1 ] || ! grep -q "^${key}=" .env 2>/dev/null; then
    if [ "$FORCE" -eq 1 ] && grep -q "^${key}=" .env 2>/dev/null; then
      tmp=$(mktemp)
      grep -v "^${key}=" .env > "$tmp"
      cat "$tmp" > .env
      rm -f "$tmp"
    fi
    if [ -s .env ] && [ -n "$(tail -c 1 .env)" ]; then
      printf '\n' >> .env
    fi
    printf '%s=%s\n' "$key" "$value" >> .env
    GENERATED="${GENERATED}${GENERATED:+ }${key}"
  fi
}

# Replace KEY's value (or append when absent); used for explicit operator input.
set_env() {
  key="$1"
  value="$2"
  tmp=$(mktemp)
  if [ -f .env ]; then
    grep -v "^${key}=" .env > "$tmp" || true
    # 防止手改过的 .env 末尾缺少换行时新旧行粘连。
    if [ -s "$tmp" ] && [ -n "$(tail -c 1 "$tmp")" ]; then
      printf '\n' >> "$tmp"
    fi
  else
    : > "$tmp"
  fi
  printf '%s=%s\n' "$key" "$value" >> "$tmp"
  cat "$tmp" > .env
  rm -f "$tmp"
}

if [ ! -f .env ]; then
  cp env.example .env
fi

ensure_env POSTGRES_USER tasktips
ensure_env POSTGRES_DATABASE tasktips
ensure_env POSTGRES_PASSWORD "$(rand_alnum 24)"
ensure_env RUSTFS_ACCESS_KEY "$(rand_alnum 20)"
ensure_env RUSTFS_SECRET_KEY "$(rand_alnum 40)"
ensure_env TASKTIPS_CURSOR_SIGNING_SECRET "$(openssl rand -hex 32)"

if [ -n "$DOMAIN" ]; then
  # Origin 精确匹配：去掉用户顺手带上的尾斜杠。
  DOMAIN="${DOMAIN%/}"
  ADMIN_ORIGIN="${ADMIN_ORIGIN%/}"
  set_env TASKTIPS_PUBLIC_BASE_URL "$DOMAIN"
  set_env TASKTIPS_ADMIN_ORIGIN "${ADMIN_ORIGIN:-$DOMAIN}"
elif [ -n "$ADMIN_ORIGIN" ]; then
  set_env TASKTIPS_ADMIN_ORIGIN "${ADMIN_ORIGIN%/}"
fi

KEY_FILE=secrets/tasktips_jwt_private_key
if [ "$FORCE" -eq 1 ] || [ ! -f "$KEY_FILE" ]; then
  mkdir -p secrets
  openssl genpkey -algorithm ed25519 -out "$KEY_FILE"
  chmod 600 "$KEY_FILE"
  # The API runs as uid 65532 inside the container; the key must stay readable
  # for it. chown works when run as root, otherwise fall back to a group/world
  # readable bit with a warning (single-admin box assumption).
  if chown 65532 "$KEY_FILE" 2>/dev/null; then
    :
  elif chmod 644 "$KEY_FILE" 2>/dev/null; then
    echo "警告: 无法 chown 到 65532，已将 $KEY_FILE 设为 644 以保证容器可读。" >&2
  fi
  GENERATED="${GENERATED}${GENERATED:+ }tasktips_jwt_private_key"
fi

if [ "$QUIET" -eq 0 ]; then
  if [ -n "$GENERATED" ]; then
    echo "已生成: $GENERATED（仅显示一次，请妥善保存 deploy/.env）"
  else
    echo "配置已存在，未做任何修改（如需重新生成请加 --force）。"
  fi
  echo "下一步:"
  echo "  docker compose --env-file deploy/.env -f deploy/compose.yaml config"
  echo "  docker compose --env-file deploy/.env -f deploy/compose.yaml pull"
  echo "  docker compose --env-file deploy/.env -f deploy/compose.yaml up -d"
fi
