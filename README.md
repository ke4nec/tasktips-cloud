# TaskTips Cloud

TaskTips 的独立同步服务、后台任务、管理后台和部署工程。第一版实施基线是
`../tasktips/docs/tasktips-server-sync-design.md`。

## 工程结构

- `crates/api`：Axum HTTP API、健康检查和指标入口。
- `crates/application`：应用用例与端口，不依赖具体传输或存储实现。
- `crates/domain`：同步领域类型，不依赖 Axum、SQLx 或 S3 SDK。
- `crates/persistence`：PostgreSQL/SQLx 适配层。
- `crates/object-store`：RustFS S3 适配层。
- `crates/worker`：快照、恢复、清理和统计任务进程。
- `contracts`：OpenAPI 3.1 唯一 HTTP 契约源及示例。
- 阶段 C 同步接口要求配置 `TASKTIPS_CURSOR_SIGNING_SECRET`；payload 通过 RustFS 内容寻址存储，
  push 前必须先完成 bootstrap。
- `admin`：Vue 3 + TypeScript 管理后台。
- `migrations/0001_init.sql`：开发阶段的单一 PostgreSQL 初始化脚本；SQLx 会从该脚本建立迁移记录。
- `deploy`：容器镜像、Compose、Caddy 和配置样例。
- `tests`：契约、集成和端到端测试。

生产自托管、升级、备份和隔离恢复演练见 [`docs/self-hosting.md`](docs/self-hosting.md)。
备份使用 [`deploy/backup.sh`](deploy/backup.sh)，恢复演练使用
[`deploy/restore-drill.sh`](deploy/restore-drill.sh)；两者都不会把密钥写入仓库。

## 本地检查

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd admin
npm ci
npm run check
```

本地启动 API：

```text
cargo run -p tasktips-api
```

默认监听 `0.0.0.0:18080`。`/health/live` 只验证进程；在 PostgreSQL 和 RustFS
不可用或配置缺失时，`/health/ready` 返回未就绪，并在依赖恢复后重新探测。

管理后台开发服务器监听 `0.0.0.0:5173`，API 请求代理到 `127.0.0.1:18080`：

```text
cd admin
npm run dev
```

管理后台 refresh Cookie 固定为 `Secure`。本机开发使用 `http://localhost:5173/admin/`；
从其他机器访问时应通过 HTTPS 反向代理，并让 `TASKTIPS_ADMIN_ORIGIN` 精确匹配浏览器 origin。

## 开发环境

PostgreSQL 和 RustFS 通过独立的开发 Compose 启动，API 与 worker 在宿主机运行：

```text
docker compose -f deploy/compose.dev.yaml up -d
cp deploy/env.dev.example deploy/.env.dev
set -a; . deploy/.env.dev; set +a
cargo run -p tasktips-api -- migrate
cargo run -p tasktips-api
```

开发栈使用固定开发凭据，只绑定 `127.0.0.1`（PostgreSQL `5432`、RustFS S3 `9000`、
控制台 `http://127.0.0.1:9001/rustfs/console`），并在启动时自动创建 `tasktips-data`
bucket。端口或凭据冲突时可用 `TASKTIPS_DEV_*` 环境变量覆盖。开发栈只用于本地开发，
不得用于生产；清空全部数据使用 `docker compose -f deploy/compose.dev.yaml down -v`。

## 完整部署环境（新服务器）

完整环境只需要 Docker Compose。`--domain` 可选，不传即 `http://localhost`
纯 HTTP；公网域名确定后再加（Caddy 自动申请证书，也可事后改 `.env` 生效）：

```text
cd deploy
bash setup.sh --domain https://example.com
docker compose pull
docker compose up -d
docker compose ps
```

`setup.sh` 一键生成全部密码与 JWT 私钥（幂等，可重复执行；生成值仅在终端显示
一次，保存在 git-ignored 的 `.env` 里）。数据库 DSN 由 Compose 用服务名自动
组装，无需手写。`migrate` 会自动先跑；RustFS bucket 由 API 自动建。

启动后创建首个管理员（无默认密码，交互式输入两次，≥12 位）：

```text
docker compose run --rm tasktips-api tasktips-api admin create --email admin@example.com
```

然后浏览器打开 `https://example.com/admin/`（或 `http://localhost/admin/`）登录。
默认使用 Docker Hub 预构建镜像（后端 `ke4nec/tasktips-cloud`、管理后台
`ke4nec/tasktips-cloud-admin`，可用 `TASKTIPS_BACKEND_IMAGE` /
`TASKTIPS_ADMIN_IMAGE` 覆盖）；本地构建改用 `docker compose up --build`。生产
部署、HTTPS 开关、备份与发布流程见 [`docs/self-hosting.md`](docs/self-hosting.md)。

不得把真实密码、JWT 私钥或 RustFS 凭据提交到 Git。
