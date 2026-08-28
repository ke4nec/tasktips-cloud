# TaskTips Cloud 实现深度审查（后端与管理平台）

## Scope

审查对象为 `tasktips-cloud` 当前 `HEAD`（`5f1579e`）以及其对应的管理平台，基准为
`../tasktips/docs/tasktips-server-sync-design.md`、`contracts/openapi.yaml` 和仓库
`AGENTS.md`。覆盖范围包括：

- Rust `domain`、`application`、`api`、`persistence`、`object-store`、`worker` 六个 crate；
- `migrations/0001`--`0029`、RLS、管理员安全视图和租约 fencing；
- OpenAPI 契约、生成客户端链路、Vue 3 管理平台；
- Compose、Caddy、备份/恢复演练脚本和 CI；
- Rust 单元/集成测试、管理平台 Vitest，以及带真实 PostgreSQL/RustFS 的 Stage B/C 流程。

原始审查只记录问题，不修改业务代码、数据库 schema 或 OpenAPI。以下结论已由本轮续修
（同日）复核；续修遵循设计基线，并同步更新 migration、OpenAPI、生成客户端和 CI。
重点仍是同步正确性、跨存储一致性、恢复可操作性、system_admin 隐私边界、管理认证和
设计文档的验收差距。

## 阶段完成度总评

| 阶段 | 状态 | 结论 |
| --- | --- | --- |
| A 仓库和契约 | 基本完成 | OpenAPI lint、客户端生成和 CI diff 已接通；同时保留 `/openapi.yaml` 兼容别名并提供设计要求的 `/api/v1/openapi.yaml`。 |
| B 认证、项目和设备 | 基本完成 | 用户邀请、登录、刷新轮换、撤销、RLS 隔离成立；管理员认证已独立，普通用户和管理员 bearer 通道已隔离。 |
| C Payload 和同步核心 | 基本完成 | change sequence、CAS、generation、cursor、成功和请求级失败幂等均已验证；bootstrap 已物化短期 manifest，容量压测仍待完成。 |
| D 桌面 ServerProvider | 不在本仓库 | 传输字段与桌面同步对象基本兼容；当前 `nextCursor` required 与实现一致，旧 review 的“契约矛盾”结论不成立。 |
| E 历史、快照和恢复 | 部分完成 | lease fencing、追加式恢复、失败后有限自动重试/显式 reopen、主动取消和 pending/孤儿快照清理成立；长期故障演练仍需在隔离环境执行（F-005、F-006）。 |
| F 管理后台 | 部分完成 | 元数据展示、邀请运营、审计导出、账号 purge 入队、恢复入队、列表分页、任务进度、同步趋势和同步页轮询可用；危险操作闭环、筛选和 N+1 优化仍缺失（F-003、F-010）。 |
| G 发布和运维 | 部分完成 | Compose、Caddy、备份脚本、健康检查、幂等保留清理、项目/账号 purge worker、安全 jobs 元数据查询和 RustFS Stage C CI 已接通；通用 jobs 定时执行、自动演练和故障注入验收未完成（F-003、F-011）。 |

**总体结论：**同步内核、RLS 隔离、管理员不具备 payload 读取能力的边界和独立管理认证目前是
本仓库最成熟的部分；公网发布前仍必须完成 purge/jobs 运维闭环和故障演练等纵深能力，并复核共享限流
在生产拓扑中的配置。按
设计基线验收，A/B/C/E 仍只能算“带已知缺口通过”，F/G 尚不可验收。

## 续修状态（2026-08-27）

本轮已落地并验证：F-001/F-002（独立 admin auth、专用 Cookie、Origin/Sec-Fetch 校验和
普通用户通道隔离）、F-004（malformed RLS identity fail-closed）、F-009（管理员 deviceId
持久化）、F-016（保留 `/openapi.yaml` 并新增设计要求的 `/api/v1/openapi.yaml`）、F-017
（system_admin bearer 拒绝普通用户路由）和 F-018（已存在对象重新做 raw-byte hash 校验）。
F-005/F-006/F-007 已分别加入显式 restore reopen、瞬时 restore 失败有限回队重试、pending/孤儿 snapshot
回收以及失败幂等响应和 30 天清理；幂等键已按设计加入 project scope，push 回放不会重复写入同步诊断。
F-008 已新增持久化 bootstrap manifest、RustFS 内容寻址对象、manifest 绑定 page token 和
过期回收；F-012 已将上传、登录、刷新、邀请、purge 和管理员 re-auth 的限流桶写入 PostgreSQL，
并为上传和账号导出临时目录增加可配置字节预算与残留清理。F-011 的数据库/RustFS 测试在 CI 缺变量时会显式失败，
Rust 纯测试 job 与外部服务测试 job 已分离。
这些项的容量和长时故障演练仍未完全覆盖。F-003
已补 re-auth nonce、邀请列表/撤销/重发和审计 CSV；项目 purge 已加入密码二次验证、deleting
状态和对象先删后清理的 worker job；账号 purge 已实现导出确认、`deleting/deleted` 状态机、数据库
脱钩、对象删除和有限重试，管理员可查询不含内部 payload 的 jobs/导出元数据。统计/快照定时 jobs、
任务分页/进度和同步趋势已接入；管理端危险操作页、轮询、筛选和批量运维仍未完成。

仍开放的主要 finding：F-010（后台轮询、筛选和 N+1 优化）、F-011（双设备 e2e、故障注入和
备份演练）、
F-013（指标粒度）、F-014（application 继续拆分）和 F-015（Argon2 生产参数备案）。这些不是
本轮验证通过就可以忽略的发布风险。

## Verified

以下行为已通过代码阅读，并由测试或数据库验证支撑：

- **序列和 CAS：** `push` 与 `execute_restore_with_lease` 在事务内锁定项目并递增
  `change_seq`，再写 `change_log`；revision head 使用 `FOR UPDATE` 比对
  `base_revision`，冲突不会 last-write-win，墓碑也走同一 CAS 路径。
- **幂等：** `pg_advisory_xact_lock` 串行化同一用户/设备/project/requestId；相同规范化
  请求回放保存的响应，不同请求体返回 `IDEMPOTENCY_CONFLICT`，请求级失败响应也会保存并在
  30 天后由 worker 清理；回放不会重复写入同步诊断。
- **Payload 跨存储顺序：**首次上传先计算 raw-byte SHA-256、校验大小和 media type，再写临时
  RustFS 对象、复制到内容寻址路径，最后在锁内登记数据库引用；清理路径在删除对象前重新
  检查数据库引用。已存在对象和 manifest 会重新流式计算 raw-byte hash；snapshot manifest
  先登记 pending，再在对象校验后转为 ready，超时 pending、bootstrap manifest 和无引用快照由
  worker 回收；项目 purge 会在删除对象前先解除数据库引用。
- **恢复 fencing：** worker 领取任务时使用 `FOR UPDATE SKIP LOCKED`，项目进入
  `maintenance`；恢复前快照、租约续期、快照 ready 和最终恢复都校验 lease token；迁移
  `0012` 会重置无 lease 的遗留 running 任务。
- **Bootstrap/Pull cursor：** HMAC-SHA256 cursor 绑定 project、owner、generation、
  sequence、kind；bootstrap 首页物化并持久化短期 manifest，page token 绑定 manifest ID，完成
  记录阻止未初始化设备直接 push；pull 响应按 OpenAPI 返回 required `nextCursor`，由 `hasMore`
  决定是否继续。
- **RLS 和角色：** `tasktips_auth/app/admin/worker` 通过 `SET LOCAL ROLE` 工作，业务表
  `FORCE ROW LEVEL SECURITY`；真实 PostgreSQL 环境中的 `identity_rls` 和 Stage B 隔离测试通过，
  malformed `app.user_id` 按 fail-closed 语义拒绝访问。
- **管理员隐私边界：** `0009_admin_safe_views.sql` 只授予运营计数、历史元数据、快照元数据
  和恢复元数据视图权限，并撤销对 payload、revision、snapshot、restore 直表的 SELECT；
  当前 admin API/DTO 不返回 Todo payload、分类/index payload、图片、hash、RustFS key、下载
  URL 或凭据，且 system_admin bearer 被普通用户 payload/sync handler 拒绝。
- **账号 purge：** `0025_account_purge.sql`/`0026_account_purge_admin_rls.sql` 增加 `deleted` 状态、导出元数据和单账号活动任务
  唯一约束。管理员先以一次性 re-auth nonce 请求最终导出，再以未过期 `exportId` 确认；worker
  先删除数据库引用和 RustFS 对象，最后保留最小用户审计行并标记 `deleted`。持久层两阶段回归
  测试已在全新 PostgreSQL 上通过，管理员只获得导出 ID/状态，不获得对象 key 或正文。
- **Token 生命周期：** refresh token 只存 SHA-256，轮换和复用检测按 family 撤销；改密码、禁用
  账号和撤设备会撤销相关会话；bearer 请求会重新检查账号和设备状态。未知用户登录会执行
  dummy Argon2，密码计算在 `spawn_blocking`。
- **对象输入校验：** content hash、Todo ID、image ID、kind/media type、单对象大小和 push
  批量上限均有服务端校验；单个对象错误不会阻断同批其它对象的逐项结果。
- **部署边界：** `/health/live` 不访问依赖，`/health/ready` 检查数据库 schema 和 bucket；
  Caddy 仅代理 `/admin`、`/api/*`、`/health/*`、`/openapi.yaml`，PostgreSQL、RustFS 和
  `/metrics` 未通过公网 Caddy 暴露；dev Compose 端口绑定 `127.0.0.1`。

## Findings

### F-001 管理认证没有独立通道（原问题已修复，阶段 F/G）

续修已将管理端切换到独立的 admin auth 路由、DTO、限流桶和 Cookie；以下为原始审查记录。

设计 §11.1 要求 `/api/v1/admin/auth/{login,refresh,logout,re-auth}`、独立 DTO、独立
限流桶和 `tasktips_admin_refresh` Cookie。实现却让管理后台调用用户的
`POST /api/v1/auth/login|refresh|logout`，并在 `token_response` 中根据 `user.role` 决定
返回 JSON refresh token 还是 Cookie（`crates/api/src/routes.rs:1057-1159,1699-1727`）。

Cookie 名为 `tasktips_refresh`，Path 为 `/api/v1/auth`（`routes.rs:1730-1764`），会随所有
用户认证请求发送；没有 re-auth nonce，也没有危险操作的二次认证边界。该问题不是单纯路由
整理，而是客户端类型、凭据作用域和安全审计边界未分离。

### F-002 管理 Cookie 请求缺 Origin/Sec-Fetch-Site 校验（原问题已修复，§14.3）

续修已对管理登录、refresh、logout 和受控写操作执行配置 origin 与 cross-site 校验；以下为原始审查记录。

`refresh` 接受 Cookie 或 JSON body 中的 refresh token，`logout` 清理同一 Cookie；这些请求
没有校验配置的管理 origin，也没有拒绝 `Sec-Fetch-Site: cross-site`。`SameSite=Strict` 只能
作为纵深措施，不能替代设计明确要求的来源校验。F-001 修复前不应将管理入口暴露到公网。

### F-003 purge、re-auth 和邀请/审计运营能力（部分修复，重要，阶段 G）

续修已实现用户密码二次验证的项目 purge、`deleting` 状态、持久化 jobs 表和 worker 的对象先删
后删数据库引用流程，并保留失败任务的有限重试。账号 purge 现在使用两阶段最终导出确认、一次性
`X-Reauth-Nonce`、账号 `deleting/deleted` 状态机，并在 worker 中完成数据库脱钩和对象清理；导出
元数据通过受限列权限和安全 view 暴露，管理员不能读取正文、hash、RustFS key 或下载 URL。仍缺
统计/快照定时任务、任务进度和管理端危险操作页；当前禁用/启用账号不能替代账号销毁流程。

### F-004 RLS 对非法 `app.user_id` 会抛 500 级数据库异常（原问题已修复，防御性安全）

迁移 `0013` 已统一使用 fail-closed 的 `app_current_user_id()`，并由 `identity_rls` 回归测试覆盖；以下为原始审查记录。

迁移策略普遍直接使用 `NULLIF(current_setting('app.user_id', true), '')::uuid`，例如
`migrations/0002_identity_projects_devices.sql:110-134`、`migrations/0004_payload_sync_core.sql:112-129`
和 `migrations/0006_history_snapshots_restore.sql:55-60`。当会话变量为非 UUID 字符串时，
PostgreSQL 在执行 RLS 表查询前抛出 `invalid input syntax for type uuid`，而不是按设计将其
视为 NULL 并拒绝访问。已在临时 PostgreSQL 中复现。当前 API 从已解析的 UUID claim 设置该
变量，因此未发现直接远程利用路径；但这是数据库会话污染/代码回归时的 500、测试契约和
纵深隔离缺口，应使用安全的 UUID 解析函数并补回归测试。

### F-005 恢复失败后缺少可操作的重开/重试闭环（部分修复，重要，可用性）

续修已提供失败后经管理员校验的 `restore-reopen`，并对同一项目的活动恢复做并发保护；瞬时 worker 失败会在
同一 maintenance 窗口内最多自动回队两次，第三次失败后才需要管理员校验。新增取消接口对排队任务立即结束，
对运行任务设置取消标记，worker 在租约校验边界安全退出并恢复项目；`deploy/restore-failure-drill.sh`
可在隔离环境模拟 lease 过期并验证接管结果。

worker 在创建恢复前快照、写 RustFS manifest 或执行恢复失败时调用
`retry_restore_job_with_lease`（`crates/worker/src/main.rs:68-125`）。迁移 `0024` 为任务增加
`attempts/run_after`，前两次失败会保留项目 `maintenance` 并延迟回队，第三次才标记 `failed`；
显式 reopen 仍是最终人工闭环。设计允许失败后保持 maintenance 以保护一致性，长期故障演练需在隔离
环境按脚本执行。

入队阶段现在在项目行锁内检查同项目 queued/running restore，并以 `Conflict` 拒绝重复请求，
因此合法请求不会形成多个 queued job；worker 领取仍按 lease 处理过期任务
（`crates/persistence/src/lib.rs:1813-1900,2030-2090`）。长期故障演练仍需按脚本在隔离环境执行。

### F-006 snapshot manifest 存在跨存储孤儿和永久 pending（部分修复，重要，数据完整性/容量）

续修已加入 pending 超时标记、manifest hash 校验和 worker 受控回收；长时故障演练仍未完成。

普通 snapshot 流程先在 `snapshot_manifest` 的数据库事务中读取并固定当前 heads，commit 后再由
API 写 RustFS，最后才调用 `record_snapshot`；若对象已写入而 DB 插入失败或进程崩溃，会留下无 DB 引用的
`snapshots/{id}/manifest.json`。反向的恢复前快照流程则在
`create_pre_restore_snapshot` 中先插入 `status='pending'` 的行，再写 RustFS
（`crates/persistence/src/lib.rs:1447-1535`）。worker 崩溃会留下 pending 行。

现有清理只扫描 `/uploads/` 和 `/payloads/sha256/`（`crates/object-store/src/lib.rs:312-390`），
而 `referenced_payload_keys` 还会把 pending snapshot key 当作受保护引用
（`crates/persistence/src/lib.rs:1116-1125`）。因此两类残留都没有可靠的回收/修复路径；需要
manifest 专用状态机、超时扫描、哈希复核和受控删除，且仍须遵守“DB 引用存在时不得删对象”。

### F-007 幂等记录不覆盖请求级失败，也没有 30 天清理（原问题已修复，§11.4/§7.1）

续修已持久化确定性请求级失败、加入 30 天过期字段和 worker 清理；以下为原始审查记录。

`idempotency_records` 只在 push 整体成功后插入（`crates/persistence/src/lib.rs:1997-2225`）。
generation mismatch、bootstrap required、请求级冲突等失败不会保存可回放的失败响应；网络
重试会重复执行校验和诊断记录，至少会产生重复的 `sync_attempts`。表也没有 `expires_at`，
worker 没有清理任务，长期运行会无限增长。应持久化成功/失败的最终响应，并按 30 天窗口
清理，同时保持 request hash 冲突语义。

### F-008 bootstrap manifest 与深分页（原问题已修复，容量压测仍待完成）

续修新增 `bootstrap_manifests`（migration `0021`），首请求在项目锁内固定 generation/
change sequence，先写入并校验 RustFS manifest，再提交数据库引用；后续页只读取固定 manifest，
page token 绑定 manifest ID、用户、项目和 generation。worker 会删除过期引用及其对象，Stage C
已覆盖两页回归。当前实现会在 API 进程内反序列化单个 manifest 后切页，大项目上线前仍需压测
manifest 大小、TTL 和内存预算，必要时再改为数据库 keyset/分块存储。

### F-009 管理平台每次登录生成新 deviceId（原问题已修复，§6.3）

续修按规范化管理员邮箱在浏览器本地持久化 deviceId；以下为原始审查记录。

`admin/src/api/client.ts:41-50` 每次调用 `login` 都使用 `crypto.randomUUID()`。管理员重复登录
会不断创建 `Unnamed device` 和 refresh family，污染设备运维数据并使撤销行为难以解释。管理
设备标识应在浏览器本地持久化，登出只撤销当前 family；若要强制新设备，应提供显式操作。

### F-010 管理平台已补分页、进度、趋势和轮询基础能力（部分修复，阶段 F）

`AdminDataView.vue` 当前主要是元数据表格，续修已接入分页控件、恢复任务进度字段、同步趋势表和同步页 15 秒轮询，仍缺少完整操作闭环：

- 恢复表单直接提交，没有目标、原因和影响范围的确认步骤，也没有单独的 restore 操作详情页；
- 只在界面提交 `targetChangeSequence`，没有 snapshotId 目标；
- 用户的项目/设备仍通过逐用户 `Promise.all` 请求，形成 N+1；筛选和批量导出仍缺失；
- Overview 仍是静态计数卡片，趋势已在 sync 视图提供成功率、冲突率和 p50/p99 延迟聚合，并自动刷新同步任务状态；
- 缺少邀请运营、purge、恢复失败处理和危险操作状态反馈。

当前英文 i18n 资源已经存在，旧 review 中“en-US 资源结构未预留”不准确；真正问题是后端
能力和交互闭环不足。

### F-011 跨边界测试与 CI 覆盖不足（重要，阶段 G）

`tests/{contract,integration,e2e}` 目录只有可配置的 smoke 脚本，跨边界主用例仍在 crate 集成测试；原始 CI 的 migration job 只运行 PostgreSQL
迁移、`identity_rls` 和 Stage B，不启动 RustFS，也不运行 Stage C；续修已让 CI 启动 RustFS 并执行
Stage C。现有环境门控测试在本地未设置连接变量时会提示跳过；若在 CI 环境缺少必需变量则显式失败，
避免“绿色但未执行”的假通过；Rust 纯测试 job 只运行 lib/bin，外部服务测试集中在 migration job。
仍缺少：OpenAPI 状态/错误兼容性测试、双设备业务数据夹具、generation/cursor 失效、
RustFS range/SigV4/孤儿回收和 admin DTO 字段 allowlist。续修已提供 payload 往返 e2e smoke、
恢复租约故障脚本和带显式隔离确认的备份/恢复演练链，但尚未在生产等价环境执行并归档结果。

### F-012 payload 上传流式化和多副本限流已修复（已修复，§8.3/§14.3）

`crates/api/src/routes.rs:455-570` 现在边读边哈希并写入受控临时文件，再由
`ObjectStore::put_payload_file` 以 `ByteStream` 流式上传；单请求上限仍为 10 MiB，避免请求体长期
驻留进程堆。上传、登录、刷新、邀请、purge 和管理员 re-auth 都有按操作分区的 PostgreSQL
固定窗口桶，worker 会回收过期桶；上传临时目录和账号导出工作区均有环境变量配置的字节上限，
并在进程启动时清理残留文件。

### F-013 可观测性粒度不足（次要，§18）

API 只有少量原子 counter（`crates/api/src/lib.rs:48-88`），缺少按错误码/操作维度的
`sync_duration_seconds` histogram、worker queue depth、数据库池指标和清理/恢复阶段耗时。
`sync_attempts` 目前主要记录 push，pull/bootstrap 的诊断覆盖不足；这会放大 F-005/F-006 的
定位成本。

### F-014 application crate 已承接共享策略，事务编排仍待继续拆分（部分修复，结构）

续修把恢复目标和账号两阶段 purge 的共享策略下沉到 `crates/application`，API 与 persistence
复用同一校验并补单元测试；框架依赖仍未进入 domain/application。较大的 purge、导出和恢复事务
编排仍在 persistence/routes 中，后续只在出现明确边界时按用例继续拆分，避免一次性增加抽象层。

### F-015 Argon2 参数已配置化并提供校准命令（已修复，§5.1/§14.1）

代码使用受约束的 Argon2id memory/time/parallelism 环境参数，PHC 哈希携带参数；
`tasktips-api admin calibrate-password` 会用固定非秘密输入测试三组候选配置，部署文档要求选择
约 150--300ms 的组合并记录生产机器结果。仓库不把某台开发机的实测值作为所有部署的结论。

### F-016 OpenAPI 文档路径与设计基线不一致（原问题已修复，契约细节）

续修新增 `/api/v1/openapi.yaml` 并保留 `/openapi.yaml` 兼容别名；以下为原始审查记录。

设计要求匿名 `GET /api/v1/openapi.yaml`（设计 §10.1、§19），实现和契约使用
`GET /openapi.yaml`，Caddy 也只代理后者（`crates/api/src/lib.rs:180-181`、`deploy/Caddyfile:18-20`、
`contracts/openapi.yaml:27`）。这不是当前客户端运行阻断，但会影响部署探活、外部文档链接和
契约测试。建议保留兼容别名后统一文档，或在新版本路径中明确迁移；不要静默收紧已发布路径。

### F-017 system_admin bearer token 未被普通用户 payload 路由拒绝（原问题已修复，安全边界）

续修在普通用户认证入口拒绝 `system_admin` bearer；以下为原始审查记录。

`authenticate` 只验证 JWT、账号和设备状态；普通项目、payload、snapshot、history 和同步
handler 没有统一拒绝 `claims.role == "system_admin"` 的守卫。因而管理员 token 可以调用普通
用户路由，在管理员名下创建/读取项目 payload；RLS 仍会阻止它读取其他用户的数据，但该行为
违反设计 §14.2“管理 token 不能调用普通用户 payload API”。应在路由层或统一授权中间件中拒绝
system_admin 使用普通用户业务通道，并增加管理员 token 访问所有 payload/sync 路由的负向测试。

### F-018 内容寻址对象命中时没有重新验证 raw-byte SHA-256（原问题已修复，数据完整性）

续修对已存在 payload 和 manifest 进行流式 raw-byte SHA-256 校验；以下为原始审查记录。

`ObjectStore::put_payload` 首次写入会校验请求字节的 SHA-256，但发现最终 key 已存在时只读取
`HEAD` 的大小并直接返回（`crates/object-store/src/lib.rs:125-137`）；`put_manifest` 对已存在
manifest 也只比较大小（`lib.rs:204-219`）。因此 RustFS 中若发生同尺寸内容损坏、误写或凭据
泄漏后的对象替换，API 仍可能把错误字节当作已验证的 content hash。该问题不是普通客户端可
直接制造的越权，但违背 raw-byte hash 的完整性契约。应对已存在对象做流式 hash/校验和复核，
或使用带强校验的不可变对象策略，并补充对象篡改/同尺寸错误对象测试。

## Schema / Contract 影响

原始审查只更新 review；本轮续修已修改 migration、OpenAPI、生成客户端和设计文档。仍需后续
决策的契约事项：

- F-003 的账号 purge 已新增端点、状态机、导出表和 worker 执行链；本轮已将项目/账号 purge
  端点、只读 jobs/导出元数据查询和对应 schema 同步到当前 OpenAPI，admin auth、re-auth、邀请运营和审计导出也已同步。
- F-016 需要决定 `/openapi.yaml` 与 `/api/v1/openapi.yaml` 的兼容策略；
- 旧 review 的 F-007（`nextCursor` required 与设计矛盾）应删除：当前 OpenAPI 与设计文字均要求
  transport DTO 带 `nextCursor`，桌面端由 `hasMore` 决定停止，不构成实现缺陷；
- F-004 的 SQL helper 和 RLS 回归已由迁移 `0013` 覆盖；F-006/F-007/F-008 若改变
  snapshot、idempotency 或 bootstrap 表结构，必须同步更新设计文档与 OpenAPI（如 HTTP 行为改变）。
- F-008 的表结构和 RustFS 命名空间已由 migration `0021` 落地；HTTP 字段保持不变，因此
  OpenAPI 只需保留现有 page token 约束。
- F-005 的有限自动重试由 migration `0024` 增加 `attempts/run_after` 支撑，任务 HTTP DTO
  保持兼容；migration `0027` 增加取消标记/状态，用户取消接口和 worker lease fencing 已同步。
- F-017 属于授权行为修正，应补充契约/集成测试，确认 system_admin 只能使用 admin API，且不改变
  普通用户错误状态映射。
- F-018 属于对象存储完整性修正，若增加校验元数据或错误状态，应同步对象存储接口说明和故障
  注入测试；不一定需要 HTTP schema 变更。

## 部署影响

- `docker compose -f deploy/compose.dev.yaml config` 通过；使用 `deploy/env.example` 的生产
  Compose 配置校验通过。实际 `deploy/.env` 不存在，因此未声称已验证真实生产 secrets。
- 临时隔离 Compose 项目已启动 PostgreSQL/RustFS，完成迁移、RLS、Stage B、Stage C 后清理；未触及
  用户已有容器或卷。
- Caddy 没有把 `/metrics` 代理到公网，这符合“内部指标端点”的方向；生产环境仍需确认 Prometheus
  在内部网络直接抓取 API。
- F-001/F-002 已改为独立 admin auth 和来源校验；Cookie 名称/Path 变更会使现有管理会话失效，
  发布时应安排一次性重新登录，并在生产环境显式设置 `TASKTIPS_ADMIN_ORIGIN`。
- F-003 的统计/快照定时 jobs、管理端危险操作页和轮询尚未实现；账号 purge 已由 worker 按对象先删、
  数据库引用后删执行，仍需长期容量和故障演练验证。
- F-005/F-006 会影响大项目恢复和 RustFS 容量预算；恢复自动回队的退避窗口、bootstrap manifest 也应在上线前做长期增长
  压测和故障注入。

## 遗留风险与建议排序

**P0：公网发布前必须完成**

1. 复核 F-001/F-002/F-017 的部署配置和跨实例限流，确保独立 admin auth、来源校验和普通
   用户通道拒绝在生产环境保持开启。

**P1：设计承诺和数据运维闭环**

1. F-003：通用 jobs 运营化、任务进度和管理端危险操作页；账号两阶段 purge 已落地，仍需长期
   worker 容量/故障演练，并将 re-auth nonce 持久化以支持多副本。
2. F-005：在隔离环境执行恢复长期故障演练；主动取消、同项目重复请求拒绝和有限自动重试已落地。
3. F-006：snapshot manifest 的哈希复核、受控清理和长时故障演练。
4. F-007：失败幂等响应的覆盖测试和过期清理压力验证。
5. F-011：真实 RustFS CI、契约/e2e、备份恢复和隐私 allowlist 测试。
6. F-012：上传流式化、临时目录容量治理和多副本限流已落地，保留部署磁盘配额监控。

**P2：容量、诊断和认证参数**

7. F-010：确认、进度轮询、snapshot 恢复、筛选和 N+1 消除；分页和趋势基础能力已落地。
8. F-013：同步趋势聚合已落地，仍需完整 histogram/worker queue 指标。
9. F-015：Argon2 参数配置和校准命令已提供，仍需各生产硬件实测备案。

**P3：结构与兼容性**

10. F-014：恢复/账号 purge 规则已下沉 application，继续拆分 persistence 和 routes 的编排。

## Verification Notes

本轮执行的验证：

- `cargo fmt --all -- --check`：通过；
- `cargo clippy --workspace --all-targets -- -D warnings`：通过；
- `cargo test --workspace -- --nocapture`：通过（本地无外部环境时，环境门控的数据库测试会明确提示跳过；CI 缺变量会失败）；另在临时
  PostgreSQL/RustFS 上实际运行并通过 `identity_rls`、`stage_b_api`、`stage_c_sync`；
- 新增迁移 `0013`--`0029` 在空数据库上通过；CI 的 Rust job 运行 lib/bin 测试，migration job 启动 RustFS
  并执行外部服务测试，缺少必需变量时会失败；
- `cd admin && npm ci && npm run check`：通过（OpenAPI lint、客户端生成、Prettier、ESLint、
  Vitest 8 tests、TypeScript build）；构建提示 JS chunk 约 1.08 MB，`npm ci` 提示
  `glob@10.5.0` deprecated，需后续依赖升级评估；
- `docker compose -f deploy/compose.dev.yaml config`：通过；使用 `deploy/env.example` 的
  生产配置 `config --quiet`：通过；实际 `deploy/.env` 缺失，未执行该文件；
- 临时数据库中直接设置 `app.user_id=not-a-uuid` 查询 RLS 表：迁移 `0013` 后按 fail-closed
  语义返回，malformed identity 回归测试通过；原始异常行为由该测试覆盖；
- 已提供 `tests/e2e/sync-smoke.sh`、`deploy/backup-drill.sh` 和 `deploy/restore-failure-drill.sh`，但尚未在隔离环境实际执行
  完整备份/恢复 drill、长时 worker 故障注入、bootstrap 大项目容量压测、浏览器截图和公网部署验证；这些仍是发布前
  残余风险，不应以本次绿色单元/集成测试替代。
