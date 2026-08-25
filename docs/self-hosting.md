# TaskTips Cloud 自托管

本文描述第一版生产部署、升级、备份和恢复演练。生产命令必须在隔离的运维主机执行，密钥不写入 Git，也不要放入 shell history。

## 首次部署

1. 安装 Docker Engine、Compose v2、`openssl`、`pg_dump`/`pg_restore` 和 MinIO `mc`。
2. 准备目录并复制示例配置：

   ```sh
   cp deploy/env.example deploy/.env
   mkdir -p deploy/secrets
   openssl genpkey -algorithm ed25519 -out deploy/secrets/tasktips_jwt_private_key
   chmod 600 deploy/secrets/tasktips_jwt_private_key
   ```

3. 修改 `deploy/.env` 中的数据库 DSN、PostgreSQL 密码、RustFS access/secret key、cursor secret 和公网 HTTPS 地址。`TASKTIPS_PUBLIC_BASE_URL` 必须与 Caddy 的公开域名一致。
4. 校验并启动：

   ```sh
   docker compose --env-file deploy/.env -f deploy/compose.yaml config
   docker compose --env-file deploy/.env -f deploy/compose.yaml up --build -d
   docker compose --env-file deploy/.env -f deploy/compose.yaml ps
   ```

`migrate` 成功前 API、worker 和 admin 不会启动。Caddy 是唯一公网入口；PostgreSQL、RustFS、`/metrics` 和容器内部端口不应映射到公网。

首次创建管理员时，在 API 容器内交互输入密码：

```sh
docker compose --env-file deploy/.env -f deploy/compose.yaml run --rm tasktips-api \
  tasktips-api admin create --email admin@example.com
```

## 升级

1. 阅读发布说明，确认迁移是 forward-only，并先完成备份。
2. 拉取固定版本代码，执行 `docker compose ... config` 和镜像构建。
3. `docker compose ... up --build -d` 会先运行迁移，再滚动启动 API、worker 和 admin。
4. 检查 `/health/ready`、`/openapi.yaml`、管理员登录、bootstrap、payload 下载和 `/metrics` 内部响应。

不要跳过迁移、回滚数据库 schema 或复用旧 JWT/cursor secret。应用回滚前必须确认新 schema 向后兼容；破坏性 API 使用新 `/api/vN` 路径。

## 备份

维护窗口内停止写入后执行：

```sh
docker compose --env-file deploy/.env -f deploy/compose.yaml scale tasktips-api=0 tasktips-worker=0
TASKTIPS_DATABASE_URL='postgres://...' RUSTFS_ENDPOINT='http://...' \
RUSTFS_BUCKET='tasktips-data' RUSTFS_ACCESS_KEY='...' RUSTFS_SECRET_KEY='...' \
  ./deploy/backup.sh /secure/backups/tasktips-$(date -u +%Y%m%dT%H%M%SZ)
```

备份包括 PostgreSQL custom dump、RustFS bucket、SHA-256 校验清单和生成时间。另行保存 JWT 私钥、Docker secret、Caddy `/data` 与 `/config` volume、部署版本记录。备份目录必须限制访问并加密保存。

## 恢复演练

每季度在隔离 PostgreSQL、RustFS 和域名环境执行一次。恢复目标不能是线上数据库或线上 bucket：

```sh
TASKTIPS_RESTORE_CONFIRM=YES \
TASKTIPS_RESTORE_DATABASE_URL='postgres://...' \
TASKTIPS_RESTORE_RUSTFS_ENDPOINT='http://...' \
TASKTIPS_RESTORE_RUSTFS_BUCKET='tasktips-data' \
TASKTIPS_RESTORE_RUSTFS_ACCESS_KEY='...' \
TASKTIPS_RESTORE_RUSTFS_SECRET_KEY='...' \
  ./deploy/restore-drill.sh /secure/backups/tasktips-YYYYmmddTHHMMSSZ
```

恢复后启动同版本 API/worker，按顺序验证：迁移、`/health/ready`、管理员登录、用户 bootstrap、payload 原始字节和 SHA-256、历史查询、快照创建、snapshot/change sequence 恢复、旧 generation push 拒绝，以及 RustFS/数据库引用完整性。记录耗时、抽样对象和结果；演练不通过前不切换生产流量。

## 监控与安全

Prometheus 只从内部网络抓取 `/metrics`。关注同步 push/conflict、restore job、payload 上传量、worker 日志和健康状态；指标标签不得包含邮箱、正文、payload hash 或精确用户/项目/设备 ID。日志中不得出现密码、token、Cookie、请求体或 RustFS 凭据。
