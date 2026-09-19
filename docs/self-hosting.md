# TaskTips Cloud 自托管

本文描述第一版生产部署、升级、备份和恢复演练。生产命令必须在隔离的运维主机执行，密钥不写入 Git，也不要放入 shell history。

## 首次部署

1. 安装 Docker Engine、Compose v2 和 `openssl`（备份/恢复另需 `pg_dump`/`pg_restore` 和 MinIO `mc`）。
2. 一键启动（本地预定义默认值开箱即用；想改用户名/密码/密钥直接编辑 `deploy/.env`）：

   ```sh
   bash deploy/quickstart.sh --domain https://example.com
   ```

   不带 `--domain` 则默认 `http://localhost`（纯 HTTP，见下文 SSL 说明）。
   首次运行会从 `deploy/env.example` 生成 `deploy/.env`（git-ignored，已存在不覆盖）
   并补齐 JWT 私钥到 `deploy/secrets/tasktips_jwt_private_key`；之后改 `.env` 对应行
   重建容器即生效。数据库 DSN 由 Compose 用服务名自动组装，不用手写。

   公网生产必须先硬化（把试用默认值换成强随机密钥）：

   ```sh
   bash deploy/setup.sh --domain https://example.com --force
   ```

   强随机密码只用字母数字（可直接拼进数据库 URL），只在本次终端输出一次。

   容器以非 root 用户（65532）运行，`deploy/secrets/tasktips_jwt_private_key`
   必须让该 uid 可读：root 下执行 setup.sh 会自动 `chown 65532`；非 root 用户
   会回退为 644 并打印警告。
3. 检查 `deploy/.env`：`TASKTIPS_PUBLIC_BASE_URL` 须与 Caddy 公开域名一致；
   `TASKTIPS_ADMIN_ORIGIN` 须精确匹配管理后台浏览器 origin（含协议、无尾斜杠）。
4. 校验并启动（默认拉取 Docker Hub 预构建镜像 `ke4nec/tasktips-cloud`（后端）
   和 `ke4nec/tasktips-cloud-admin`（管理后台），可用 `TASKTIPS_BACKEND_IMAGE` /
   `TASKTIPS_ADMIN_IMAGE` 覆盖）：

   ```sh
   docker compose --env-file deploy/.env -f deploy/compose.yaml config
   docker compose --env-file deploy/.env -f deploy/compose.yaml pull
   docker compose --env-file deploy/.env -f deploy/compose.yaml up -d
   docker compose --env-file deploy/.env -f deploy/compose.yaml ps
   ```

   离线或定制构建时把 `pull` 换成 `up --build -d`，Compose 会改用
   `deploy/Dockerfile`（后端：API、worker、迁移）和 `deploy/Dockerfile.admin`
  （管理后台）在本地构建同样的两个镜像。

## SSL 开关与密钥找回

默认不启用 SSL：`TASKTIPS_PUBLIC_BASE_URL` 为 `http` 开头时 Caddy 只提供纯
HTTP，不申请证书。需要 HTTPS 时把它改为 `https://你的域名`（DNS 须解析到本机
并放行 80/443），重启 Caddy 后自动完成 ACME 申请与续期；改回 `http` 即关闭。

密码类配置统一保存在 `deploy/.env`（`POSTGRES_PASSWORD`、`RUSTFS_ACCESS_KEY` /
`RUSTFS_SECRET_KEY`、`TASKTIPS_CURSOR_SIGNING_SECRET`）；生效后的完整配置可用
`docker compose --env-file deploy/.env -f deploy/compose.yaml config` 查看。
密码不会出现在任何容器日志里，这是故意的——日志会被轮转和采集，不适合做密钥存储。

注意：管理后台 refresh Cookie 固定带 `Secure` 标记。明文 HTTP 下只有经
`localhost` 访问的浏览器会接受该 Cookie；用局域网 IP 明文访问时登录本身可用，
但 refresh 续期会被浏览器拒绝（约 15 分钟后需重新登录）。需要长期稳定的内网
访问请同样走 HTTPS（内网域名 + 自签/内网 CA，或直接用 `localhost` 跳板）。

## 镜像发布

`.github/workflows/docker-publish.yml` 在 `master` 分支推送和 `v*` 标签推送时自动构建后端、admin 两个镜像并推送到 Docker Hub（`master` 发布 `latest`/`master`/`sha-<commit>`，`vX.Y.Z` 额外发布 `X.Y.Z`/`X.Y`）；Pull Request 只做构建验证，不推送。

发布前需要在仓库设置中配置 Secrets `DOCKERHUB_USERNAME`（Docker Hub 账号或组织名）和 `DOCKERHUB_TOKEN`（Access Token，不要用真实密码）；镜像仓库默认为 `ke4nec/tasktips-cloud`（后端）和 `ke4nec/tasktips-cloud-admin`（管理后台），可用 Actions 变量 `DOCKERHUB_BACKEND_REPOSITORY` / `DOCKERHUB_ADMIN_REPOSITORY` 覆盖。

构建使用 BuildKit Actions 缓存（`type=gha,mode=max`）复用 cargo/npm 层加速后续构建，不上传任何 artifact；任务结束时会裁剪 builder 缓存和悬空镜像，避免 runner 磁盘被中间产物占满。

`migrate` 成功前 API、worker 和 admin 不会启动。Caddy 是唯一公网入口；PostgreSQL、RustFS、`/metrics` 和容器内部端口不应映射到公网。

当前开发阶段的数据库结构收敛在 `migrations/0001_init.sql`。首次部署或重建空库由
`tasktips-api migrate` 执行该脚本；已有完整的旧分段迁移结构会被识别为同一最终结构并继续使用，
不会改写业务数据。

上传临时文件使用 `TASKTIPS_UPLOAD_TEMP_DIR`，每个 API 实例以
`TASKTIPS_UPLOAD_TEMP_MAX_BYTES` 限制同时占用的字节数（默认 512 MiB）；进程启动时会清理超过
一小时的残留文件。账号导出使用 `TASKTIPS_EXPORT_TMP_DIR`，单个导出临时工作区受
`TASKTIPS_EXPORT_TMP_MAX_BYTES` 限制（默认 2 GiB），worker 启动时清理超过一天的残留工作区。
上传、项目/账号 purge 的固定窗口桶写入 PostgreSQL，多个 API 副本共享同一限流状态。

密码使用 Argon2id，参数由 `TASKTIPS_ARGON2_MEMORY_KIB`、`TASKTIPS_ARGON2_TIME_COST` 和
`TASKTIPS_ARGON2_PARALLELISM` 配置，默认值为 19456/2/1，并限制在受支持范围内。部署前可运行
`tasktips-api admin calibrate-password`，在目标机器上比较固定测试输入的耗时；选择约 150--300ms
的组合后，将参数写入部署环境。PHC 哈希会携带参数，后续升级应保留旧哈希验证能力并在改密时使用新参数。

首次创建管理员时，在 API 容器内交互输入密码：

```sh
docker compose --env-file deploy/.env -f deploy/compose.yaml run --rm tasktips-api \
  tasktips-api admin create --email admin@example.com
```

## 升级

1. 阅读发布说明，确认迁移是 forward-only，并先完成备份。
2. 拉取固定版本代码，执行 `docker compose ... config` 和镜像构建。
3. `docker compose ... up --build -d` 会先运行迁移，再滚动启动 API、worker 和 admin。
4. 检查 `/health/ready`、`/api/v1/openapi.yaml`（以及兼容的 `/openapi.yaml`）、管理员登录、bootstrap、payload 下载和 `/metrics` 内部响应。管理员登录、refresh、logout 和危险操作必须使用 `/api/v1/admin/*` 路径；发布后现有管理员会话需要重新登录。

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

长期 worker 故障演练必须在上述隔离环境中先让一个 restore 进入 `running`，再运行：

```sh
TASKTIPS_DRILL_CONFIRM=YES \
TASKTIPS_DRILL_DATABASE_URL='postgres://...' \
TASKTIPS_DRILL_API_URL='https://isolated.example.test' \
TASKTIPS_DRILL_ACCESS_TOKEN='...' \
TASKTIPS_DRILL_PROJECT_ID='...' \
TASKTIPS_DRILL_RESTORE_ID='...' \
  ./deploy/restore-failure-drill.sh
```

脚本只会让目标 restore 的 lease 过期来模拟 worker 崩溃，然后轮询到 `succeeded`、`failed` 或
`cancelled`；不会读取或打印 payload，也不会连接生产数据库。恢复任务不再需要时，用户可调用
`POST /api/v1/projects/{projectId}/restores/{restoreId}/cancel`；排队任务立即取消，运行任务在
下一个 lease 校验边界安全退出并重新开放项目。

恢复后启动同版本 API/worker，按顺序验证：迁移、`/health/ready`、管理员登录、用户 bootstrap、payload 原始字节和 SHA-256、历史查询、快照创建、snapshot/change sequence 恢复、旧 generation push 拒绝，以及 RustFS/数据库引用完整性。记录耗时、抽样对象和结果；演练不通过前不切换生产流量。

payload PUT 会先边读边哈希并写入操作系统临时目录，再以流式方式提交 RustFS；单请求上限为 10 MiB。
生产环境应监控临时目录空间并设置容器/主机清理策略，避免并发上传耗尽本地磁盘。

项目永久清除必须通过 `POST /api/v1/projects/{projectId}/purge` 提交当前用户密码和原因。接口只
创建异步任务并立即将项目置为 `deleting`；worker 会先解除数据库引用再删除 RustFS 对象，失败任务
按有限次数重试。

账号永久清除必须由管理员在独立管理通道中先调用
`POST /api/v1/admin/users/{userId}/purge`（`confirmed=false`）生成最终导出，再使用返回的
`exportId`、`confirmed=true` 和一次性 `X-Reauth-Nonce` 确认。导出由 worker 写入私有 RustFS
对象，管理 API 只返回任务/导出 ID 和状态，不提供正文、对象 key、下载 URL 或凭据。确认后账号进入
`deleting`，worker 先解除全部项目、设备和会话引用，再删除对象，成功后保留最小审计引用并标记为
`deleted`；失败会保持 `deleting`，只能重试或恢复任务。不能用直接 SQL 删除替代该流程。

## 监控与安全

Prometheus 只从内部网络抓取 `/metrics`。关注同步 push/conflict、restore job、payload 上传量、worker 日志和健康状态；指标标签不得包含邮箱、正文、payload hash 或精确用户/项目/设备 ID。日志中不得出现密码、token、Cookie、请求体或 RustFS 凭据。
