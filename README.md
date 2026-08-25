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
- `admin`：Vue 3 + TypeScript 管理后台。
- `migrations`：SQLx migration。
- `deploy`：容器镜像、Compose、Caddy 和配置样例。
- `tests`：契约、集成和端到端测试。

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

默认监听 `127.0.0.1:8080`。`/health/live` 只验证进程；在 PostgreSQL 和 RustFS
适配尚未接入启动流程前，`/health/ready` 会按设计返回未就绪。

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

## 完整部署环境

完整环境需要 Docker Compose：

```text
cd deploy
Copy-Item env.example .env
docker compose up --build
```

不得把真实密码、JWT 私钥或 RustFS 凭据提交到 Git。

