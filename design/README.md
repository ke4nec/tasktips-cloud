# TaskTips Cloud 管理后台 UI 设计

这套设计基于当前 `admin` 源码和 HTTP 契约梳理功能，沿用 TaskTips 桌面客户端的 Fluent 配色与交互语言。交付内容是可点击的离线设计稿，所有记录均为合成示例。

直接用浏览器打开 [index.html](index.html)。无需启动后端、安装依赖或联网。也可以在仓库根目录运行 `python3 -m http.server 8766 --bind 127.0.0.1`，访问 `http://127.0.0.1:8766/design/`。

## 交付文件

| 文件                                       | 内容                                                                       |
| ------------------------------------------ | -------------------------------------------------------------------------- |
| [index.html](index.html)                   | 总览、用户、项目、设备、同步诊断、审计、设置、登录、注册，以及设计规范页面 |
| [analysis.md](analysis.md)                 | 当前功能清单、页面问题、API 对照及未接入能力                               |
| [specification.md](specification.md)       | 配色、布局、组件、恢复流程、异常状态和落地约束                             |
| [screenshots/](screenshots/)               | 浅色 / 深色页面、恢复确认、账号禁用、加载 / 空白 / 异常状态截图            |
| [client-theme.css](client-theme.css)       | 从客户端 `src/styles/theme.css` 复制的主题令牌                             |
| [styles.css](styles.css)                   | 管理界面的布局与组件规范                                                   |
| [app.js](app.js)                           | vue-i18n 文案、示例视图数据和演示交互                                      |
| [review.mjs](review.mjs)                   | 浏览器布局检查、交互检查和截图导出脚本                                     |
| [review-results.json](review-results.json) | 最近一次浏览器检查结果                                                     |

## 如何评审

1. 从总览开始，切换 1 / 7 / 30 天查看趋势。左侧导航切换全部业务页面。
2. 点击顶部月亮 / 太阳图标切换明暗主题；主题选择保存在独立的设计稿 localStorage 键中。
3. 在用户页搜索、翻页、筛选待审核账号，体验新建、批准、禁用及目标 ID 校验。
4. 打开项目详情，查看元数据时间线，进入「发起恢复」。填写目标和原因后，输入完整项目 ID，提交后进入恢复任务列表。
5. 在设备、同步诊断和审计页打开详情抽屉；审计页可导出当前筛选下的示例 CSV。
6. 在设置页切换注册开关，再从「设计规范 → 申请账号」查看注册开放 / 关闭状态。注册成功后显示等待审核；普通用户账号与管理员登录明确区分。
7. 顶部状态选择器可预览业务页的加载、空白、异常状态；登录页提供提交中、登录错误演示。

设计稿中的账号、注册设置和任务仅在本次页面会话内变化，刷新后还原。账号及密码不会发出请求或写入持久存储。顶部工具栏、示例数据标注和设计规范入口属于评审工具，不进入正式产品。

## 主要截图

| 场景              | 浅色                                               | 深色                                              |
| ----------------- | -------------------------------------------------- | ------------------------------------------------- |
| 总览 · 1440px     | [查看](screenshots/overview-1440-light.png)        | [查看](screenshots/overview-1440-dark.png)        |
| 用户管理 · 1440px | [查看](screenshots/users-1440-light.png)           | [查看](screenshots/users-1440-dark.png)           |
| 项目管理 · 1440px | [查看](screenshots/projects-1440-light.png)        | [查看](screenshots/projects-1440-dark.png)        |
| 同步诊断 · 1440px | [查看](screenshots/sync-1440-light.png)            | [查看](screenshots/sync-1440-dark.png)            |
| 管理登录 · 1440px | [查看](screenshots/login-1440-light.png)           | [查看](screenshots/login-1440-dark.png)           |
| 恢复确认          | [查看](screenshots/restore-confirm-1440-light.png) | [查看](screenshots/restore-confirm-1440-dark.png) |
| 总览 · 1280px     | [查看](screenshots/overview-1280-light.png)        | [查看](screenshots/overview-1280-dark.png)        |

## 验证与影响

浏览器检查已通过：10 个页面 × 1440 / 1280 / 390px × 浅色 / 深色，共 60 组布局检查及 21 项交互检查；检查页面水平溢出、标题、翻译、图片和按钮名称。共导出 37 张截图，浏览器错误与远程请求均为 0。截图采用真实 Chromium 渲染，整页截图保留纵向滚动内容，弹层截图采用固定视口。

本次仅新增 `design/`，没有更改生产 admin、API、生成客户端、数据库或部署配置。因此不涉及契约 / schema / 部署变更，也不需要启动 Docker。原型的浏览器检查不代表生产认证、后端恢复任务或 API 集成已经验证。

复现浏览器检查：使用 **Node.js 22+** 和独立的 Chromium 进程（可通过 `CHROME_BIN` 指定本机 Chromium / Chrome 可执行文件）。

```bash
"$CHROME_BIN" --headless --disable-gpu --disable-dev-shm-usage \
  --no-first-run --no-default-browser-check \
  --remote-debugging-port=9236 \
  --user-data-dir=/tmp/tasktips-cloud-design-review about:blank
```

在另一终端执行：

```bash
node design/review.mjs
```

该脚本连接独立浏览器中的第一个普通页面，导出截图并覆盖检查结果。不要把调试端口指向日常使用的浏览器。端口可通过 `DESIGN_CDP_PORT` 覆盖。

格式与脚本语法检查：

```bash
node --check design/app.js
node --check design/review.mjs
cd design
node ../admin/node_modules/prettier/bin/prettier.cjs --check .
```

离线依赖来自现有 `admin/node_modules`：Vue 3.5.41、vue-i18n 11.4.9，保留 MIT 许可证于 `vendor/`。品牌图标复制自客户端 `src-tauri/icons/128x128.png`。图表、图标和登录插图均由 SVG / CSS 构建，没有 CDN、远程字体或 AI 生成图片依赖。
