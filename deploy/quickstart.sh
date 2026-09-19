#!/bin/sh
# 本地试用一键启动：预定义默认值开箱即用，无需手写密码。
#
#   bash deploy/quickstart.sh [--domain https://example.com]
#
# 实际执行（都在 deploy/ 目录下）：
#   1. 没有 .env 就从 env.example 复制（含本地默认用户名/密码/密钥，可自行修改）；
#   2. 调 setup.sh 补齐 JWT 私钥（幂等，已存在不覆盖；--domain 会同步写 PUBLIC_BASE_URL）；
#   3. docker compose up -d 拉起全部 7 个服务（migrate 自动先跑）。
#
# 改配置：编辑 .env 对应行后 `docker compose --env-file .env -f compose.yaml up -d` 重建即可。
# 公网生产：不要用默认密钥，改跑 `bash deploy/setup.sh --domain https://example.com --force`
# 换成强随机密钥后再 up（见 docs/self-hosting.md）。
set -eu

DEPLOY_DIR=$(dirname "$0")
cd "$DEPLOY_DIR"

if [ ! -f .env ]; then
  cp env.example .env
  echo "已从 env.example 生成 .env（本地默认值，可自行修改用户名/密码）"
fi

# 补 JWT 私钥 + 可选 --domain；已存在的密码/密钥不会被覆盖。
bash setup.sh --quiet "$@"

# 公网 https + 默认试用密钥只提醒、不断流程。
public_url=$(grep -E '^TASKTIPS_PUBLIC_BASE_URL=' .env 2>/dev/null | tail -n 1 | cut -d= -f2-)
if printf '%s' "$public_url" | grep -q '^https://'; then
  case "$public_url" in
    https://localhost*|https://127.0.0.1*|"") ;;
    *)
      if grep -qx 'POSTGRES_PASSWORD=tasktips-dev' .env 2>/dev/null \
        || grep -qx 'RUSTFS_ACCESS_KEY=tasktips-dev' .env 2>/dev/null \
        || grep -qx 'RUSTFS_SECRET_KEY=tasktips-dev-secret' .env 2>/dev/null \
        || grep -qx 'TASKTIPS_CURSOR_SIGNING_SECRET=dev-only-cursor-signing-secret-32-bytes' .env 2>/dev/null; then
        echo "警告: 公网地址 $public_url 仍在使用默认试用密钥，生产请先跑 bash deploy/setup.sh --domain $public_url --force 换强密钥。" >&2
      fi
      ;;
  esac
fi

docker compose --env-file .env -f compose.yaml up -d
docker compose --env-file .env -f compose.yaml ps

echo "下一步（首次）：创建管理员并登录 http://localhost/admin/："
echo "  docker compose --env-file .env -f compose.yaml run --rm tasktips-api tasktips-api admin create --email admin@example.com"
