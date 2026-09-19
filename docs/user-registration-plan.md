# 用户注册与账号管理计划（已实现）

> 落地差异（以实现为准）：`refresh` 轮换本就经 `lock_active_user` 校验状态，无需补；
> `pending→active` 审批复用既有 `POST .../enable`（放宽一处状态迁移）；
> 注册请求体为 `{email, password}`（不下发 token，故不需要 `deviceId`）；
> 开关读写直连 `owner` 池（`instance_settings` 无 RLS/授权）。


## 1. 目标

1. 管理员可在管理后台直接创建用户（邮箱 + 密码，立即可用）。
2. 普通用户可自助注册（邮箱 + 密码，不做密码重置）。
3. 自助注册的账号默认为 `pending`，须管理员激活后才能登录/同步。
4. 管理后台可禁用/启用用户。
5. 管理后台可开关“开放注册”；关闭时注册接口与注册页面同时停用。

非目标：密码重置/找回、邮箱验证码、第三方登录、用户名（非邮箱）登录——账号标识沿用现有 `email` 字段，不新增列、不做 `0002` 迁移（理由见 §3）。

## 2. 现状复用（不重复造轮子）

| 现状 | 位置 | 复用方式 |
|---|---|---|
| `users.status` 五态：`active/disabled/pending/deleting/deleted`，默认 `pending` | `migrations/0001_init.sql:24` | 注册写 `pending`，激活写 `active`，禁用写 `disabled`；无需 DDL |
| 非 `active` 账号已无法登录/同步（`lock_active_user`） | `crates/persistence/src/lib.rs:5245` | 注册/禁用天然生效，只需确认 `pending` 同样被挡（已有测试则补断言） |
| 管理员禁用/启用接口已存在 | `POST /api/v1/admin/users/{userId}/disable\|enable`（`routes.rs:2279`） | 后端不动，只补管理后台 UI 按钮 |
| 邀请制开户已存在（7 天过期、一次性 `token`、限流） | `POST /api/v1/admin/invitations`、`POST /api/v1/auth/invitations/activate` | 保留；管理员直建用户作为第二条开户路径 |
| `instance_settings(key, value jsonb)` 通用开关表已存在 | `migrations/0001_init.sql:473` | 注册开关存 `registration.enabled`（bool），无需 DDL |
| 管理后台用户列表接口已存在（不含密码/hash） | `GET /api/v1/admin/users` → `admin_user_metadata` 视图 | 列表页直接加状态列与操作列 |
| 固定窗口限流桶（邀请激活已用） | `distributed_rate_limit_buckets` | 注册接口复用同一模式，按 IP 限流 |

## 3. 关键决策

- **D1 账号标识用邮箱（已确认）**：登录、邀请、`RLS`、对象键全部围绕 `email_normalized` 构建；不加 `username` 列。
- **D2 注册开关默认关**：单机自托管默认是私用，`instance_settings` 无记录时按关闭处理；管理员在后台打开后才可注册。
- **D3 加法兼容，不升 `/api/vN`**：新增均为新路径/新字段，按仓库规则不算破坏性变更；`openapi.yaml` 同步增补并跑契约测试。

## 4. 功能设计

### F1 管理员直建用户

- 新增 `POST /api/v1/admin/users`（`system_admin`，`Origin` 校验 + 审计）：body `{email, password}`，密码复用 `valid_password`（≥12 位）与 `Argon2id`；直接写 `status='active'`，返回用户元数据（不回显任何 hash）。
- 与邀请制并存：需要用户自己设密码走邀请，需要立即给号走直建。
- 管理后台用户页加“新建用户”对话框（邮箱 + 密码两次确认）。

### F2 自助注册

- 新增公开 `POST /api/v1/auth/register`（`security: []`）：body `{email, password, deviceId}`，成功写 `status='pending'` 并返回“等待管理员审核”提示（已确认：不下发可用 token；`pending` 账号登录保持拒绝并提示等待激活）。
- 限流：复用邀请激活的双层限流（进程内 `InvitationActivation` 桶 + `distributed_rate_limit_buckets` 按 IP），防批量刷号。
- 邮箱唯一冲突返回 `409`，不泄露“该邮箱是否已存在”以外的信息（与邀请激活一致即可）。

### F3 待激活管理

- `GET /api/v1/admin/users` 已含 `status`，管理后台用户表加状态筛选（`pending` 置顶或独立 tab）。
- 激活复用 `POST .../enable`（需确认 `set_account_status` 允许 `pending→active`，不允许则放宽该迁移）；同时复用它做 F4 的启用。激活后用户即可以正常登录。
- 审计：`create/register/approve` 全部写 `audit_events`。

### F4 禁用/启用

- 后端已有；管理后台用户行加“禁用/启用”按钮 + 二次确认（`purge` 的危险操作样式已有，可照抄交互）。
- 语义：`disabled` 立即踢下线——需确认登出/刷新轮换路径同样走 `lock_active_user`（登录已挡；`refresh` 轮换要补查：若只查了 token 未查状态，需加状态校验并加回归测试）。

### F5 注册开关

- `instance_settings` 新增键 `registration.enabled`：`PUT /api/v1/admin/settings/registration {enabled}` + `GET` 回显（`system_admin`，审计）。
- `POST /api/v1/auth/register` 每次读取该键（无记录视为关），关闭时返回 `403 REGISTRATION_DISABLED`。
- 注册页面（见 §5）打开时先读一个公开只读端点（如 `GET /api/v1/auth/registration-status {enabled}`，`security: []`，只返回 bool）决定渲染表单还是“注册已关闭”。

## 5. 注册页面

- 落在管理后台 SPA 新增公开路由 `/admin/register`（`Caddy` 的 `handle_path /admin/*` 与 `nginx` 的 `try_files ... /index.html` 已支持，无需改网关）。
- 页面逻辑：挂载即调 `registration-status`；开 → 邮箱/密码/确认表单，成功提示“等待管理员激活”；关 → 静态提示页。
- `vue-i18n` 中英（默认 `zh-CN`）文案；`adminDeviceId(email)` 复用登录页的设备标识逻辑。

## 6. 契约与迁移影响

- `contracts/openapi.yaml`：新增 `RegisterRequest/Response`、`CreateUserRequest/AdminUser` 复用、`RegistrationSettings`、`registration-status` schema 与 4 个路径；`info.version` 不变。
- DB 迁移：**无**（开关走现有 `instance_settings`，状态走现有枚举）。若后续要用户名列，再立 `0002` 前向迁移。
- 管理员会话：新增路径不影响旧 token；`enable/disable` 已要求 `/api/v1/admin/*` 路径，保持即可。

## 7. 安全要点（评审必查）

- `pending/disabled` 账号的登录、`refresh` 轮换、同步 `bootstrap/pull/push` 全链路拒绝，已有 `lock_active_user` 的补齐 `refresh` 路径检查。
- 注册/激活限流与邀请同级；错误信息不区分“邮箱已存在/不存在”之外的细节。
- 管理后台返回与日志永不含密码、`token`、邀请 `token` 明文（创建邀请的 `token` 仅创建响应返回一次，与现状一致）。
- `registration-status` 公开端点只返回 bool，不暴露其他配置。

## 8. 测试矩阵

- 注册开/关：开→`pending` 落库；关→`403`；无记录→视为关。
- `pending` 登录/`refresh`/同步全拒；激活后全通；禁用后 `refresh` 续期失败。
- 重复注册同一邮箱→`409`；弱密码→`400`。
- 开关切换即时生效（无缓存或缓存 TTL 短）。
- `RLS`：A 用户看不到 B 的项目；`admin` 元数据不含 hash。
- 管理后台：`client.test.ts` 补新端点；`auth` store 补注册页状态机。

## 9. 实施步骤

1. 后端：`POST /api/v1/auth/register` + `GET /api/v1/auth/registration-status` + `POST /api/v1/admin/users` + `GET/PUT /api/v1/admin/settings/registration`，及 `refresh` 路径状态校验补齐。
2. 契约：`openapi.yaml` 增补 + 生成客户端 + 契约测试。
3. 前端：`RegisterView` + 路由 + 用户表状态列/激活/禁用启用按钮 + 新建用户对话框 + 注册开关设置项 + `i18n`。
4. 测试：§8 矩阵 + `cargo test --workspace` + `npm run check`。
5. 文档：`self-hosting.md` 补“开放注册运营”小节（默认关、审批入口、滥用时一键关闭）。

## 10. 文件清单（预估）

- `crates/api/src/routes.rs`、`crates/api/src/lib.rs`（路由注册）
- `crates/persistence/src/lib.rs`（`register_user`、`get/set_registration_enabled`）
- `crates/application/src/...`（用例层校验，按现有分层习惯）
- `contracts/openapi.yaml` + 生成客户端
- `admin/src/views/RegisterView.vue`、`admin/src/router.ts`、`admin/src/api/client.ts`、`admin/src/views/AdminDataView.vue`、`admin/src/i18n.ts`
