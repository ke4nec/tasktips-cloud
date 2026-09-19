/* Offline visual prototype. All records below are synthetic presentation fixtures,
 * not API DTOs. Production types continue to come from contracts/openapi.yaml. */
(() => {
  const {
    createApp,
    ref,
    computed,
    watch,
    nextTick,
    onMounted,
    onBeforeUnmount,
  } = Vue;
  const { createI18n, useI18n } = VueI18n;
  const messages = {
    'zh-CN': {
      design: {
        label: '管理后台 · UI 设计稿',
        mock: '交互原型 · 全部为示例数据',
        state: '预览页面状态',
        theme: '切换浅色 / 深色',
        specs: '设计规范',
      },
      brand: {
        name: 'TaskTips Cloud',
        console: '管理控制台',
        workspace: '个人云服务',
        selfHosted: '自托管服务',
        admin: 'Administrator',
        role: '系统管理员',
      },
      nav: {
        main: '主导航',
        workspace: '工作空间',
        operations: '服务运维',
        overview: '总览',
        users: '用户管理',
        projects: '项目管理',
        devices: '设备管理',
        sync: '同步诊断',
        audit: '审计日志',
        settings: '设置',
        specs: '设计规范',
        login: '管理员登录',
        register: '申请账号',
      },
      eyebrow: {
        overview: 'WORKSPACE OVERVIEW',
        users: 'ACCOUNT MANAGEMENT',
        projects: 'PROJECT METADATA',
        devices: 'CONNECTED DEVICES',
        sync: 'SYNC DIAGNOSTICS',
        audit: 'AUDIT TRAIL',
        settings: 'PREFERENCES',
        specs: 'DESIGN LANGUAGE',
      },
      title: {
        overview: '服务总览',
        users: '用户管理',
        projects: '项目管理',
        devices: '设备管理',
        sync: '同步诊断',
        audit: '审计日志',
        settings: '设置',
        specs: '熟悉的 TaskTips，延续到云端。',
      },
      description: {
        overview: '掌握服务运行情况，让每一次同步都有迹可循。',
        users: '管理账号与注册申请，控制云端服务的访问权限。',
        projects: '查看项目运行状态、变更历史与恢复记录。',
        devices: '查看已注册设备、客户端版本与最近活动。',
        sync: '从同步结果到恢复任务，快速定位运行异常。',
        audit: '关键管理操作，全程留痕。',
        settings: '管理服务访问规则，调整你的工作环境。',
        specs: '沿用桌面客户端的 Fluent 配色、灰阶层次与轻量交互。',
      },
      privacy: {
        title: '隐私优先',
        short: '仅管理元数据，内容始终私有。',
        health: '仅显示连接状态，不显示存储配置。',
        footer: '只见运行状态，内容始终私有',
        settings:
          '管理后台只展示账号、设备和运行元数据。项目历史与恢复操作不会展示用户内容。',
      },
      common: {
        refresh: '刷新',
        viewAll: '查看全部',
        details: '详情',
        manage: '管理',
        close: '关闭',
        previous: '上一页',
        next: '下一页',
        page: '第 {n} 页',
        never: '尚未登录',
        pageFilter: '筛选当前页',
        mockSearch: '搜索示例项目',
        sampleRows: '示例集合 · {n} 条记录',
        updated: '更新于 {time}',
        retry: '重新加载',
        reset: '重置筛选',
        disabled: '不可用',
        reason: '操作原因',
        back: '上一步',
        cancel: '取消',
        confirm: '确认',
        refreshed: '示例数据已刷新',
        saved: '演示设置已更新',
        copied: '已复制',
        required: '请完整填写表单，原因不能只包含空格。',
        idMismatch: '输入的 ID 与操作目标不一致，请重新核对。',
        invalidPassword: '密码需至少 12 位，并包含字母和数字。',
        passwordMismatch: '两次输入的密码不一致。',
        noExport: '暂无可导出的示例记录。',
      },
      states: {
        normal: '默认状态',
        loading: '加载中',
        empty: '空状态',
        error: '异常状态',
        emptyTitle: '这里还没有记录',
        emptyBody:
          '服务产生记录后，会显示在这里。你也可以重置筛选查看已有数据。',
        errorTitle: '暂时无法获取数据',
        errorBody: '请检查服务连接后重试。若问题持续，可通过请求 ID 排查。',
        noMatch: '没有找到匹配记录',
      },
      metrics: {
        users: '用户总数',
        projects: '项目总数',
        devices: '已注册设备',
        storage: '存储用量',
        activeHint: '个已启用账号',
        projectsHint: '所有账号下的同步项目',
        devicesHint: '累计注册设备数',
        storageHint: '已引用的内容容量',
        active: '已启用账号',
        revisions: '历史版本',
        tombstones: '删除标记',
        restores: '待处理恢复',
        queue: '队列中',
      },
      trend: {
        title: '同步趋势',
        subtitle: '每日成功请求 · UTC 自然日',
        days: '{n} 天',
        rate: '同步成功率',
        total: '共 {n} 次请求',
        succeeded: '成功请求',
        conflicts: '冲突请求',
        details: '查看诊断',
        chartLabel: '最近 {n} 天每日成功同步请求数',
      },
      health: {
        live: 'API 在线',
        title: '服务状态',
        ready: '服务已就绪',
        checked: '最近检查于 10:24:36',
        api: 'API 进程',
        database: '数据库连接',
        storage: '对象存储连接',
        online: '在线',
        connected: '已连接',
      },
      operations: { recent: '最近操作', subtitle: '同步与恢复任务的最新动态' },
      col: {
        account: '账号',
        role: '角色',
        status: '状态',
        created: '创建时间',
        lastLogin: '最后登录',
        actions: '操作',
        project: '项目',
        owner: '所属用户',
        generation: '代际',
        sequence: '变更序号',
        updated: '更新时间',
        device: '设备',
        platform: '平台',
        version: '客户端版本',
        lastSeen: '最近活动',
        operation: '操作类型',
        items: '对象数',
        latency: '耗时',
        errorCode: '错误码',
        time: '时间',
        event: '事件',
        actor: '操作者',
        subject: '目标用户',
        request: '请求 ID',
        date: '日期',
        attempts: '请求数',
        succeeded: '成功数',
        conflicts: '冲突数',
        p50: 'P50 延迟',
        p99: 'P99 延迟',
        id: 'ID',
        deviceId: '设备 ID',
        reason: '操作原因',
        retries: '尝试次数',
        runAfter: '下次运行',
        cancelRequested: '已请求取消',
        revokedAt: '撤销时间',
        lastPull: '最后拉取',
        lastPush: '最后推送',
        userId: '用户 ID',
        projectId: '项目 ID',
      },
      status: {
        active: '正常',
        pending: '待审核',
        disabled: '已禁用',
        deleting: '清除中',
        deleted: '已清除',
        maintenance: '维护中',
        revoked: '已撤销',
        succeeded: '成功',
        failed: '失败',
        conflict: '冲突',
        queued: '排队中',
        running: '执行中',
        cancelled: '已取消',
      },
      role: { user: '普通用户', system_admin: '系统管理员' },
      filter: {
        all: '全部',
        pending: '待审核',
        active: '已启用',
        disabled: '已禁用',
        sync: '同步记录',
        restore: '恢复任务',
        allStatus: '全部状态',
      },
      search: {
        users: '搜索本页邮箱或用户 ID',
        projects: '搜索项目名称或 ID',
        devices: '搜索本页设备名称或 ID',
        sync: '搜索本页项目 ID、操作或错误码',
        audit: '搜索本页事件或请求 ID',
      },
      users: {
        create: '新建用户',
        approve: '批准',
        enable: '启用',
        disable: '禁用',
        related: '关联资源',
        relatedHint: '查看该账号的项目与设备元数据。',
        confirmId: '输入完整用户 ID 以确认禁用',
        updated: '演示账号状态已更新',
        created: '演示用户已创建',
        duplicate: '示例集合中已存在此邮箱。',
      },
      projects: {
        note: '在项目详情中查看历史元数据或发起恢复。恢复任务可在同步诊断中跟踪。',
        history: '最近变更',
        historyHint: '仅展示版本、变更序号与设备等元数据。',
        revision: '更新版本',
        tombstone: '删除标记',
        metadataOnly: '历史记录不包含用户内容，也不提供内容预览或下载。',
      },
      sync: {
        window: '当前所选时间范围',
        p50: 'P50 同步延迟',
        p99: 'P99 同步延迟',
        latestDay: '最近一日 · 09/19 UTC',
        polling: '每 15 秒刷新 · 演示',
        daily: '每日同步统计',
        dailyHint: '展示所选范围最近 3 日；分位数按日独立计算。',
      },
      operation: {
        push: '推送',
        pull: '拉取',
        restore: '项目恢复',
        project_purge: '项目清除',
      },
      audit: {
        export: '导出示例 CSV',
        note: '审计事件永久保留。导出包含操作元数据，不包含用户内容。',
        exported: '已导出当前筛选下的示例审计记录',
      },
      auditAction: {
        account: { enabled: '批准 / 启用账号', disabled: '禁用账号' },
        admin: { login: '管理员登录' },
        restore: { requested: '发起项目恢复' },
        registration: { updated: '更新注册设置' },
        user: { created: '新建用户' },
      },
      settings: {
        access: '账号与访问',
        registration: '开放自助注册',
        registrationHint: '允许新用户申请账号。审核通过后，账号才可使用。',
        approval: '新注册账号默认进入待审核状态。',
        appearance: '外观与显示',
        theme: '界面主题',
        themeHint: '与 TaskTips 客户端保持一致的明暗体验。',
        light: '浅色',
        dark: '深色',
        system: '跟随系统',
        language: '界面语言',
        languageHint: '设计稿默认使用简体中文。',
        zh: '简体中文',
        about: '关于服务',
        version: '应用版本',
        mode: '部署方式',
        audit: '查看操作审计',
        closeImpact: '关闭后，新用户将无法自助申请。已有账号不受影响。',
        openImpact: '开放后，新账号可提交注册申请，仍需管理员审核。',
      },
      specs: {
        family: '同一套颜色，同一种熟悉感。',
        familyHint: '从客户端主题令牌出发，为管理场景重新组织信息密度。',
        palette: '语义色板',
        type: '文字层级',
        body: '账号、项目与服务运行信息',
        components: '组件与状态',
        radius: '面板 8px · 控件 5px',
        pages: '页面索引',
        stateHint:
          '顶部工具栏可切换主题及加载、空白、异常状态。所有操作仅作用于当前示例。',
      },
      color: {
        background: '工作区背景',
        surface: '内容面板',
        accent: '主要操作',
        text: '主要文字',
        success: '正常状态',
        danger: '危险操作',
      },
      auth: {
        signOut: '退出登录',
        eyebrow: 'YOUR TASKS. YOUR CLOUD.',
        hero1: '让同步井然有序，',
        hero2: '让内容始终私有。',
        heroDescription:
          '为 TaskTips 提供一个安静、可靠的云端空间。在熟悉的界面里，轻松管理你的服务。',
        trust: '自托管 · 元数据管理 · 操作可追溯',
        registered: '申请已提交',
        registeredHint:
          '账号现处于待审核状态，请等待管理员批准。普通用户通过 TaskTips 客户端登录。',
        backToLogin: '返回管理登录',
        closed: '自助注册暂未开放',
        closedHint: '请联系服务管理员创建账号，或稍后再试。',
        adminAccess: '管理控制台',
        join: '加入 TaskTips Cloud',
        welcome: '欢迎回来',
        createAccount: '申请云端账号',
        loginHint: '使用系统管理员账号登录，管理你的云端服务。',
        registerHint: '注册普通用户账号，审核通过后即可连接客户端。',
        email: '邮箱地址',
        emailPlaceholder: '输入邮箱地址',
        password: '密码',
        passwordPlaceholder: '输入密码',
        confirmPassword: '确认密码',
        showPassword: '显示或隐藏密码',
        passwordHint: '至少 12 位，包含字母和数字。',
        signIn: '登录管理后台',
        submitRegister: '提交注册申请',
        noAccount: '需要客户端账号？',
        hasAccount: '已有管理员账号？',
        register: '申请账号',
        demo: '这是离线交互设计稿，请勿输入真实账号或密码。',
        failed: '登录未成功，请检查账号、密码及管理员权限。',
      },
      drawer: {
        user: 'ACCOUNT DETAILS',
        project: 'PROJECT DETAILS',
        device: 'DEVICE DETAILS',
        operation: 'OPERATION DETAILS',
        audit: 'AUDIT EVENT',
      },
      restore: {
        start: '发起恢复',
        taskStatus: '任务状态',
        taskHint:
          '恢复在后台异步执行。离开页面不会中断任务；此处仅显示任务状态，不推算完成百分比。',
        failure:
          '恢复失败后，项目可能仍处于维护状态。请先完成引用完整性校验，再由授权流程重新开放同步。',
        target: '选择恢复目标',
        review: '确认影响',
        type: '恢复方式',
        sequence: '目标变更序号',
        snapshot: '使用已有快照 ID',
        snapshotId: '快照 ID',
        reasonPlaceholder: '说明恢复原因，操作将记入审计日志',
        impact:
          '执行恢复时，项目将进入维护状态，暂停同步写入。系统先创建恢复前快照，再生成新的版本与代际；客户端需要重新同步。',
        confirmId: '输入完整项目 ID 以确认恢复',
        next: '下一步，确认影响',
        submit: '确认并提交恢复',
        queued: '演示恢复任务已排队，可在恢复任务列表查看',
        invalidSequence: '请输入 0 到当前变更序号之间的整数。',
      },
      modal: {
        create: '新建用户',
        enable: '批准 / 启用账号',
        disable: '禁用账号',
        restore: '恢复项目',
        registration: '修改注册设置',
        logout: '退出管理后台',
      },
      modalHint: {
        create: '新建一个普通用户账号，用于连接 TaskTips 客户端。',
        enable: '启用后，该账号可以登录并使用同步服务。',
        disable: '禁用后，该账号将无法继续访问服务。操作会写入审计日志。',
        restore: '选择一个历史状态，将项目恢复到该时刻。',
        registration: '变更立即生效，请确认对新账号申请的影响。',
        logout: '确认退出当前管理会话。',
      },
      boolean: { true: '是', false: '否' },
    },
  };

  const iconPaths = {
    cloud:
      '<path d="M7 18h10a4 4 0 0 0 .7-7.94A6 6 0 0 0 6.1 8.7 4.7 4.7 0 0 0 7 18Z"/>',
    grid: '<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>',
    users:
      '<circle cx="9" cy="8" r="3"/><path d="M3 20v-2a6 6 0 0 1 12 0v2M16 5a3 3 0 0 1 0 6m2 3a5 5 0 0 1 3 4v2"/>',
    'user-plus':
      '<circle cx="9" cy="8" r="3"/><path d="M3 20v-2a6 6 0 0 1 12 0v2m3-13v6m-3-3h6"/>',
    folder:
      '<path d="M3 7V5a2 2 0 0 1 2-2h5l3 3h6a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z"/><path d="M3 8h18"/>',
    monitor:
      '<rect x="3" y="3" width="18" height="13" rx="2"/><path d="M8 21h8m-4-5v5"/>',
    sync: '<path d="M20 8a8 8 0 0 0-14-3L3 8m0-5v5h5M4 16a8 8 0 0 0 14 3l3-3m0 5v-5h-5"/>',
    refresh: '<path d="M20 7v5h-5M20 12a8 8 0 1 0-2 6"/>',
    audit:
      '<rect x="5" y="3" width="14" height="18" rx="2"/><path d="M9 7h6m-6 5h6m-6 5h4"/>',
    settings:
      '<path d="m10 3-.7 2.3-2 .9L5 5.7 3 9.2l1.7 1.8v2L3 14.8 5 18.3l2.3-.5 2 .9L10 21h4l.7-2.3 2-.9 2.3.5 2-3.5-1.7-1.8v-2L21 9.2 19 5.7l-2.3.5-2-.9L14 3Z"/><circle cx="12" cy="12" r="3"/>',
    shield:
      '<path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6l8-3Z"/><path d="m9 12 2 2 4-4"/>',
    logout:
      '<path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4m7 14 5-5-5-5M9 12h12"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.5 1.5m11 11L19 19M5 19l1.5-1.5m11-11L19 5"/>',
    moon: '<path d="M20.5 14a8.6 8.6 0 0 1-10.5-10.5 9 9 0 1 0 10.5 10.5Z"/>',
    'arrow-up-right': '<path d="M6 18 18 6M6 6h12v12"/>',
    'arrow-right': '<path d="M4 12h16m-6-6 6 6-6 6"/>',
    'chevron-right': '<path d="m9 5 7 7-7 7"/>',
    'chevron-left': '<path d="m15 5-7 7 7 7"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    search: '<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/>',
    check: '<path d="m5 12 4 4L19 6"/>',
    history: '<path d="M3 10a9 9 0 1 1 2 8M3 4v6h6m3-4v6l4 2"/>',
    activity: '<path d="M2 12h5l3-9 4 18 3-9h5"/>',
    storage:
      '<ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v14c0 4 16 4 16 0V5M4 12c0 4 16 4 16 0"/>',
    alert:
      '<path d="m10.3 4-8 14a2 2 0 0 0 1.7 3h16a2 2 0 0 0 1.7-3l-8-14a2 2 0 0 0-3.4 0Z"/><path d="M12 9v4m0 4v.1"/>',
    inbox: '<path d="M3 12 6 4h12l3 8v8H3Zm0 0h5l2 3h4l2-3h5"/>',
    info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10v.1"/>',
    download: '<path d="M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5"/>',
    lock: '<rect x="4" y="10" width="16" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3m-4 5v2"/>',
    eye: '<path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/>',
    close: '<path d="m6 6 12 12M6 18 18 6"/>',
    clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  };
  const Icon = {
    props: ['name'],
    computed: {
      paths() {
        return iconPaths[this.name] || iconPaths.info;
      },
    },
    template:
      '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true" v-html="paths"></svg>',
  };
  const Badge = {
    props: ['value'],
    setup() {
      return useI18n();
    },
    computed: {
      tone() {
        return (
          {
            active: 'success',
            succeeded: 'success',
            pending: 'warning',
            maintenance: 'warning',
            conflict: 'warning',
            running: 'info',
            queued: 'info',
            failed: 'danger',
            deleting: 'danger',
          }[this.value] || ''
        );
      },
    },
    template:
      '<span class="badge" :class="tone">{{ t("status." + value) }}</span>',
  };

  const uid = (n) =>
    `f8a2c${String(n).padStart(3, '0')}-6d41-4c9a-8e5b-0100000000${String(n).padStart(2, '0')}`;
  const pid = (n) =>
    `c3b7e${String(n).padStart(3, '0')}-728a-45b1-b6c4-0200000000${String(n).padStart(2, '0')}`;
  const did = (n) =>
    `d2a8f${String(n).padStart(3, '0')}-582b-42b1-b6c4-0300000000${String(n).padStart(2, '0')}`;
  const app = createApp({
    setup() {
      const { t } = useI18n();
      const validPages = [
        'overview',
        'users',
        'projects',
        'devices',
        'sync',
        'audit',
        'settings',
        'specs',
        'login',
        'register',
      ];
      const readPage = () =>
        validPages.includes(location.hash.slice(1))
          ? location.hash.slice(1)
          : 'overview';
      const page = ref(readPage());
      const previewState = ref('normal');
      const themeMode = ref('light');
      const theme = ref('light');
      const systemTheme = matchMedia('(prefers-color-scheme: dark)');
      const applyTheme = () => {
        theme.value =
          themeMode.value === 'system'
            ? systemTheme.matches
              ? 'dark'
              : 'light'
            : themeMode.value;
        document.documentElement.dataset.theme = theme.value;
      };
      const setTheme = (mode) => {
        themeMode.value = mode;
        applyTheme();
        try {
          localStorage.setItem('tasktips-cloud-design-theme', mode);
        } catch {
          /* File preview may disable storage. */
        }
      };
      try {
        const stored = localStorage.getItem('tasktips-cloud-design-theme');
        if (['light', 'dark', 'system'].includes(stored))
          themeMode.value = stored;
      } catch {
        /* Preview is usable without storage. */
      }
      applyTheme();
      const toggleTheme = () =>
        setTheme(theme.value === 'light' ? 'dark' : 'light');
      const isAuth = computed(() => ['login', 'register'].includes(page.value));
      const navigation = [
        {
          label: 'nav.workspace',
          items: [
            { id: 'overview', icon: 'grid' },
            { id: 'users', icon: 'users' },
            { id: 'projects', icon: 'folder' },
            { id: 'devices', icon: 'monitor' },
          ],
        },
        {
          label: 'nav.operations',
          items: [
            { id: 'sync', icon: 'activity' },
            { id: 'audit', icon: 'audit' },
          ],
        },
      ];
      const users = ref([
        {
          id: uid(1),
          email: 'admin@example.test',
          role: 'system_admin',
          status: 'active',
          createdAt: '2026/08/12',
          lastLoginAt: '09/19 10:20',
          tint: 'blue',
        },
        {
          id: uid(2),
          email: 'lin@example.test',
          role: 'user',
          status: 'active',
          createdAt: '2026/08/18',
          lastLoginAt: '09/19 10:18',
          tint: 'mint',
        },
        {
          id: uid(3),
          email: 'yue@example.test',
          role: 'user',
          status: 'pending',
          createdAt: '2026/09/19',
          lastLoginAt: null,
          tint: 'lavender',
        },
        {
          id: uid(4),
          email: 'chen@example.test',
          role: 'user',
          status: 'pending',
          createdAt: '2026/09/18',
          lastLoginAt: null,
          tint: 'amber',
        },
        {
          id: uid(5),
          email: 'zhou@example.test',
          role: 'user',
          status: 'active',
          createdAt: '2026/08/26',
          lastLoginAt: '09/18 21:36',
          tint: 'blue',
        },
        {
          id: uid(6),
          email: 'xu@example.test',
          role: 'user',
          status: 'disabled',
          createdAt: '2026/08/30',
          lastLoginAt: '09/12 16:08',
          tint: 'mint',
        },
        {
          id: uid(7),
          email: 'he@example.test',
          role: 'user',
          status: 'active',
          createdAt: '2026/09/02',
          lastLoginAt: '09/18 17:45',
          tint: 'lavender',
        },
        {
          id: uid(8),
          email: 'tang@example.test',
          role: 'user',
          status: 'active',
          createdAt: '2026/09/06',
          lastLoginAt: '09/19 09:22',
          tint: 'amber',
        },
      ]);
      const projects = ref([
        {
          id: pid(1),
          ownerUserId: uid(2),
          name: '个人空间',
          generation: 3,
          changeSequence: 8426,
          status: 'active',
          updatedAt: '09/19 10:24',
        },
        {
          id: pid(2),
          ownerUserId: uid(5),
          name: '默认项目',
          generation: 2,
          changeSequence: 3618,
          status: 'active',
          updatedAt: '09/19 10:22',
        },
        {
          id: pid(3),
          ownerUserId: uid(7),
          name: '桌面同步',
          generation: 4,
          changeSequence: 6219,
          status: 'maintenance',
          updatedAt: '09/19 10:18',
        },
        {
          id: pid(4),
          ownerUserId: uid(8),
          name: '个人项目',
          generation: 1,
          changeSequence: 956,
          status: 'active',
          updatedAt: '09/19 09:46',
        },
        {
          id: pid(5),
          ownerUserId: uid(6),
          name: '默认项目',
          generation: 2,
          changeSequence: 1240,
          status: 'disabled',
          updatedAt: '09/12 16:08',
        },
        {
          id: pid(6),
          ownerUserId: uid(2),
          name: '备用空间',
          generation: 1,
          changeSequence: 386,
          status: 'active',
          updatedAt: '09/18 19:21',
        },
      ]);
      const devices = ref([
        {
          id: did(1),
          ownerUserId: uid(2),
          displayName: 'Lin 的笔记本',
          platform: 'Windows',
          appVersion: '0.1.0',
          status: 'active',
          lastSeenAt: '09/19 10:24',
          lastPullAt: '09/19 10:24',
          lastPushAt: '09/19 10:23',
          revokedAt: null,
        },
        {
          id: did(2),
          ownerUserId: uid(5),
          displayName: 'MacBook Air',
          platform: 'macOS',
          appVersion: '0.1.0',
          status: 'active',
          lastSeenAt: '09/19 10:22',
          lastPullAt: '09/19 10:22',
          lastPushAt: '09/19 10:21',
          revokedAt: null,
        },
        {
          id: did(3),
          ownerUserId: uid(7),
          displayName: '桌面电脑',
          platform: 'Windows',
          appVersion: '0.1.0',
          status: 'active',
          lastSeenAt: '09/19 10:18',
          lastPullAt: '09/19 10:18',
          lastPushAt: '09/19 10:16',
          revokedAt: null,
        },
        {
          id: did(4),
          ownerUserId: uid(8),
          displayName: 'Ubuntu Desktop',
          platform: 'Linux',
          appVersion: '0.1.0',
          status: 'active',
          lastSeenAt: '09/19 09:46',
          lastPullAt: '09/19 09:46',
          lastPushAt: '09/19 09:45',
          revokedAt: null,
        },
        {
          id: did(5),
          ownerUserId: uid(6),
          displayName: '旧笔记本',
          platform: 'Windows',
          appVersion: '0.1.0',
          status: 'revoked',
          lastSeenAt: '09/12 16:08',
          lastPullAt: '09/12 16:08',
          lastPushAt: '09/12 16:07',
          revokedAt: '09/12 17:00',
        },
        {
          id: did(6),
          ownerUserId: uid(2),
          displayName: 'Mac mini',
          platform: 'macOS',
          appVersion: '0.1.0',
          status: 'active',
          lastSeenAt: '09/18 19:21',
          lastPullAt: '09/18 19:21',
          lastPushAt: '09/18 19:20',
          revokedAt: null,
        },
      ]);
      const operations = ref([
        {
          id: 'op-demo-0924',
          operation: 'push',
          status: 'succeeded',
          projectId: pid(1),
          deviceId: did(1),
          itemCount: 12,
          latencyMs: 42,
          errorCode: null,
          time: '09/19 10:24:32',
          attempts: 1,
          runAfter: null,
          cancelRequested: false,
        },
        {
          id: 'op-demo-0923',
          operation: 'pull',
          status: 'succeeded',
          projectId: pid(2),
          deviceId: did(2),
          itemCount: 8,
          latencyMs: 28,
          errorCode: null,
          time: '09/19 10:23:18',
          attempts: 1,
          runAfter: null,
          cancelRequested: false,
        },
        {
          id: 'op-demo-0922',
          operation: 'push',
          status: 'conflict',
          projectId: pid(1),
          deviceId: did(6),
          itemCount: 1,
          latencyMs: 36,
          errorCode: 'REVISION_CONFLICT',
          time: '09/19 10:22:46',
          attempts: 1,
          runAfter: null,
          cancelRequested: false,
        },
        {
          id: 'op-demo-0921',
          operation: 'restore',
          status: 'running',
          projectId: pid(3),
          deviceId: null,
          itemCount: null,
          latencyMs: null,
          errorCode: null,
          time: '09/19 10:18:04',
          attempts: 1,
          runAfter: null,
          cancelRequested: false,
        },
        {
          id: 'op-demo-0920',
          operation: 'push',
          status: 'failed',
          projectId: pid(4),
          deviceId: did(4),
          itemCount: 3,
          latencyMs: 1024,
          errorCode: 'STORAGE_UNAVAILABLE',
          time: '09/19 09:45:16',
          attempts: 1,
          runAfter: null,
          cancelRequested: false,
        },
        {
          id: 'op-demo-0919',
          operation: 'restore',
          status: 'queued',
          projectId: pid(6),
          deviceId: null,
          itemCount: null,
          latencyMs: null,
          errorCode: null,
          time: '09/19 09:42:08',
          attempts: 0,
          runAfter: '09/19 10:25:00',
          cancelRequested: false,
        },
        {
          id: 'op-demo-0918',
          operation: 'restore',
          status: 'succeeded',
          projectId: pid(2),
          deviceId: null,
          itemCount: 24,
          latencyMs: null,
          errorCode: null,
          time: '09/18 18:36:12',
          attempts: 1,
          runAfter: null,
          cancelRequested: false,
        },
      ]);
      const audit = ref([
        {
          id: 1284,
          action: 'account.enabled',
          actorUserId: uid(1),
          subjectUserId: uid(8),
          projectId: null,
          requestId: 'req_demo_7c91',
          createdAt: '09/19 10:21:08',
          reason: '批准注册申请',
        },
        {
          id: 1283,
          action: 'admin.login',
          actorUserId: uid(1),
          subjectUserId: null,
          projectId: null,
          requestId: 'req_demo_7c90',
          createdAt: '09/19 10:20:02',
          reason: null,
        },
        {
          id: 1282,
          action: 'restore.requested',
          actorUserId: uid(1),
          subjectUserId: uid(7),
          projectId: pid(3),
          requestId: 'req_demo_7c89',
          createdAt: '09/19 10:18:04',
          reason: '应用户请求恢复历史状态',
        },
        {
          id: 1281,
          action: 'registration.updated',
          actorUserId: uid(1),
          subjectUserId: null,
          projectId: null,
          requestId: 'req_demo_7c88',
          createdAt: '09/18 16:38:24',
          reason: '开放自助注册',
        },
        {
          id: 1280,
          action: 'account.disabled',
          actorUserId: uid(1),
          subjectUserId: uid(6),
          projectId: null,
          requestId: 'req_demo_7c87',
          createdAt: '09/12 17:00:08',
          reason: '应用户请求暂停账号',
        },
        {
          id: 1279,
          action: 'user.created',
          actorUserId: uid(1),
          subjectUserId: uid(7),
          projectId: null,
          requestId: 'req_demo_7c86',
          createdAt: '09/02 09:10:16',
          reason: null,
        },
      ]);
      const mainMetrics = [
        {
          label: 'metrics.users',
          value: '128',
          icon: 'users',
          extra: '116',
          hint: 'metrics.activeHint',
        },
        {
          label: 'metrics.projects',
          value: '186',
          icon: 'folder',
          hint: 'metrics.projectsHint',
        },
        {
          label: 'metrics.devices',
          value: '243',
          icon: 'monitor',
          hint: 'metrics.devicesHint',
        },
        {
          label: 'metrics.storage',
          value: '8.42',
          unit: 'GiB',
          icon: 'storage',
          hint: 'metrics.storageHint',
        },
      ];
      const secondaryMetrics = [
        { label: 'metrics.active', value: '116' },
        { label: 'metrics.revisions', value: '24,816' },
        { label: 'metrics.tombstones', value: '386' },
        { label: 'metrics.restores', value: '2', tag: 'metrics.queue' },
      ];
      const healthItems = [
        { label: 'health.api', status: 'health.online' },
        { label: 'health.database', status: 'health.connected' },
        { label: 'health.storage', status: 'health.connected' },
      ];
      const period = ref(7);
      const dailyCounts = [
        826, 1025, 934, 1315, 1104, 1236, 956, 1508, 1329, 1400, 1098, 1230,
        1518, 1280, 1563, 1740, 1630, 1349, 1590, 1726, 1416, 1530, 1764, 1326,
        1668, 1587, 1930, 1668, 1889, 2092,
      ];
      const trends = dailyCounts.map((n, i) => ({
        day: new Date(Date.UTC(2026, 7, 21 + i)).toISOString().slice(0, 10),
        attempts: n + 8 + (i % 5),
        succeeded: n,
        conflicts: 5 + (i % 4),
        p50LatencyMs: i === 29 ? 42 : 35 + (i % 10),
        p99LatencyMs: i === 29 ? 186 : 162 + (i % 30),
      }));
      const visibleTrends = computed(() => trends.slice(-period.value));
      const trendStats = computed(() => {
        const total = visibleTrends.value.reduce(
          (a, r) => ({
            attempts: a.attempts + r.attempts,
            succeeded: a.succeeded + r.succeeded,
            conflicts: a.conflicts + r.conflicts,
          }),
          { attempts: 0, succeeded: 0, conflicts: 0 },
        );
        return {
          ...total,
          rate: ((100 * total.succeeded) / total.attempts).toFixed(2),
        };
      });
      const chartPoints = computed(() =>
        visibleTrends.value.map((r, i, arr) => ({
          x: arr.length === 1 ? 395 : 44 + (i * 702) / (arr.length - 1),
          y: 150 - (r.succeeded / 2400) * 132,
        })),
      );
      const chartLine = computed(() =>
        chartPoints.value
          .map((p, i) => `${i ? 'L' : 'M'}${p.x},${p.y}`)
          .join(' '),
      );
      const chartArea = computed(
        () =>
          `${chartLine.value} L${chartPoints.value.at(-1).x},150 L${chartPoints.value[0].x},150 Z`,
      );
      const search = ref('');
      const filter = ref('all');
      const pageNumber = ref(1);
      const pageSize = 6;
      const rowSource = computed(
        () =>
          ({
            users: users.value,
            projects: projects.value,
            devices: devices.value,
            sync: operations.value,
            audit: audit.value,
          })[page.value] || [],
      );
      const filteredRows = computed(() =>
        rowSource.value.filter((row) => {
          const groupMatch =
            filter.value === 'all' ||
            (filter.value === 'sync'
              ? ['pull', 'push'].includes(row.operation)
              : filter.value === 'restore'
                ? row.operation === 'restore'
                : row.status === filter.value);
          const query = search.value.trim().toLowerCase();
          return (
            groupMatch &&
            (!query ||
              Object.values(row).some((value) =>
                String(value ?? '')
                  .toLowerCase()
                  .includes(query),
              ) ||
              (row.action && t('auditAction.' + row.action).includes(query)))
          );
        }),
      );
      const pagedRows = computed(() =>
        filteredRows.value.slice(
          (pageNumber.value - 1) * pageSize,
          pageNumber.value * pageSize,
        ),
      );
      const columnKeys = {
        users: ['account', 'role', 'status', 'created', 'lastLogin'],
        projects: [
          'project',
          'owner',
          'status',
          'generation',
          'sequence',
          'updated',
        ],
        devices: [
          'device',
          'owner',
          'platform',
          'version',
          'status',
          'lastSeen',
        ],
        sync: [
          'operation',
          'project',
          'status',
          'items',
          'latency',
          'errorCode',
          'time',
        ],
        audit: ['event', 'actor', 'subject', 'request', 'created'],
      };
      const tableColumns = computed(() =>
        (columnKeys[page.value] || []).map((key) => ({
          key,
          numeric: ['generation', 'sequence', 'items', 'latency'].includes(key),
        })),
      );
      const setFilter = (value) => {
        filter.value = value;
        pageNumber.value = 1;
      };
      watch([search, filter], () => {
        pageNumber.value = 1;
      });
      const number = (value) => new Intl.NumberFormat('zh-CN').format(value);
      const shortId = (value) => {
        if (!value) return '—';
        return /^[0-9a-f]{8}-/i.test(value)
          ? value.slice(0, 8)
          : value.replace('-demo-', '-');
      };
      const updatedAt = ref('10:24:36');
      const toast = ref('');
      let toastTimer;
      let refreshTimer;
      const notify = (key) => {
        clearTimeout(toastTimer);
        toast.value = t(key);
        toastTimer = setTimeout(() => {
          toast.value = '';
        }, 3600);
      };
      const refresh = () => {
        updatedAt.value = new Date().toLocaleTimeString('zh-CN', {
          hour12: false,
        });
        notify('common.refreshed');
      };
      const registration = ref(true);
      const drawerDialog = ref(null);
      const modalDialog = ref(null);
      const drawer = ref(null);
      const modal = ref(null);
      const restoreStep = ref(1);
      const form = ref({});
      const formError = ref('');
      const closeDrawer = () => {
        drawerDialog.value.close();
        drawer.value = null;
      };
      const closeModal = () => {
        modalDialog.value.close();
        modal.value = null;
        form.value = {};
        formError.value = '';
      };
      const showDrawer = async (type, row) => {
        drawer.value = { type, row };
        await nextTick();
        drawerDialog.value.showModal();
      };
      const openModal = async (type, row = null) => {
        form.value = {
          email: '',
          password: '',
          confirmPassword: '',
          confirmId: '',
          reason: '',
          targetType: 'sequence',
          sequence: row?.changeSequence ?? 0,
          snapshotId: '',
        };
        formError.value = '';
        restoreStep.value = 1;
        modal.value = { type, row };
        await nextTick();
        modalDialog.value.showModal();
      };
      const dismissBackdrop = (event, type) => {
        if (event.target !== event.currentTarget) return;
        const r = event.currentTarget.getBoundingClientRect();
        if (
          event.clientX < r.left ||
          event.clientX > r.right ||
          event.clientY < r.top ||
          event.clientY > r.bottom
        )
          (type === 'drawer' ? closeDrawer : closeModal)();
      };
      const go = (target) => {
        location.hash = target;
      };
      const goRelated = (target, id) => {
        closeDrawer();
        go(target);
        setTimeout(() => {
          search.value = id;
        }, 0);
      };
      const startRestore = (row) => {
        closeDrawer();
        openModal('restore', row);
      };
      const drawerTitle = computed(() => {
        if (!drawer.value) return '';
        const { type, row } = drawer.value;
        return type === 'user'
          ? row.email
          : type === 'project'
            ? row.name
            : type === 'device'
              ? row.displayName
              : type === 'operation'
                ? t('operation.' + row.operation)
                : t('auditAction.' + row.action);
      });
      const drawerDetails = computed(() => {
        if (!drawer.value) return [];
        const { type, row } = drawer.value;
        const details = [];
        const add = (key, value, mono = false) =>
          details.push({ label: 'col.' + key, value, mono });
        add('id', row.id, true);
        if (type === 'user') {
          add('role', t('role.' + row.role));
          add('created', row.createdAt);
          add('lastLogin', row.lastLoginAt || t('common.never'));
        }
        if (type === 'project') {
          add('owner', row.ownerUserId, true);
          add('generation', row.generation);
          add('sequence', number(row.changeSequence));
          add('updated', row.updatedAt);
        }
        if (type === 'device') {
          add('owner', row.ownerUserId, true);
          add('platform', row.platform);
          add('version', row.appVersion, true);
          add('lastSeen', row.lastSeenAt);
          add('lastPull', row.lastPullAt);
          add('lastPush', row.lastPushAt);
          add('revokedAt', row.revokedAt);
        }
        if (type === 'operation') {
          add('projectId', row.projectId, true);
          add('deviceId', row.deviceId, true);
          add('time', row.time);
          add('items', row.itemCount);
          add('latency', row.latencyMs === null ? null : row.latencyMs + ' ms');
          add('errorCode', row.errorCode, true);
          if (row.operation === 'restore') {
            add('retries', row.attempts);
            add('runAfter', row.runAfter);
            add('cancelRequested', t('boolean.' + row.cancelRequested));
          }
        }
        if (type === 'audit') {
          add('actor', row.actorUserId, true);
          add('subject', row.subjectUserId, true);
          add('projectId', row.projectId, true);
          add('request', row.requestId, true);
          add('time', row.createdAt);
          add('reason', row.reason);
        }
        return details;
      });
      let auditCounter = 1300;
      const addAudit = (
        action,
        subjectUserId = null,
        projectId = null,
        reason = null,
      ) => {
        audit.value.unshift({
          id: auditCounter++,
          action,
          actorUserId: uid(1),
          subjectUserId,
          projectId,
          requestId: `req_demo_${auditCounter}`,
          createdAt:
            '09/19 ' +
            new Date().toLocaleTimeString('zh-CN', { hour12: false }),
          reason,
        });
      };
      const validPassword = (value) =>
        typeof value === 'string' &&
        value.length >= 12 &&
        /[a-z]/i.test(value) &&
        /[0-9]/.test(value);
      const submitModal = () => {
        formError.value = '';
        const { type, row } = modal.value;
        const f = form.value;
        if (
          ['enable', 'disable', 'restore'].includes(type) &&
          !f.reason.trim()
        ) {
          formError.value = t('common.required');
          return;
        }
        if (type === 'restore') {
          if (
            f.targetType === 'sequence' &&
            (f.sequence === '' ||
              !Number.isSafeInteger(Number(f.sequence)) ||
              Number(f.sequence) < 0 ||
              Number(f.sequence) > row.changeSequence)
          ) {
            formError.value = t('restore.invalidSequence');
            return;
          }
          if (restoreStep.value === 1) {
            restoreStep.value = 2;
            return;
          }
          if (f.confirmId.trim() !== row.id) {
            formError.value = t('common.idMismatch');
            return;
          }
          operations.value.unshift({
            id: `restore-demo-${operations.value.length + 1}`,
            operation: 'restore',
            status: 'queued',
            projectId: row.id,
            deviceId: null,
            itemCount: null,
            latencyMs: null,
            errorCode: null,
            time:
              '09/19 ' +
              new Date().toLocaleTimeString('zh-CN', { hour12: false }),
            attempts: 0,
            runAfter: null,
            cancelRequested: false,
          });
          addAudit(
            'restore.requested',
            row.ownerUserId,
            row.id,
            f.reason.trim(),
          );
          closeModal();
          go('sync');
          setTimeout(() => setFilter('restore'), 0);
          notify('restore.queued');
          return;
        }
        if (type === 'create') {
          if (!validPassword(f.password)) {
            formError.value = t('common.invalidPassword');
            return;
          }
          if (f.password !== f.confirmPassword) {
            formError.value = t('common.passwordMismatch');
            return;
          }
          if (
            users.value.some(
              (u) => u.email.toLowerCase() === f.email.trim().toLowerCase(),
            )
          ) {
            formError.value = t('users.duplicate');
            return;
          }
          const user = {
            id: uid(users.value.length + 1),
            email: f.email.trim(),
            role: 'user',
            status: 'active',
            createdAt: '2026/09/19',
            lastLoginAt: null,
            tint: 'blue',
          };
          users.value.unshift(user);
          addAudit('user.created', user.id);
          closeModal();
          search.value = '';
          setFilter('all');
          notify('users.created');
          return;
        }
        if (['enable', 'disable'].includes(type)) {
          if (type === 'disable' && f.confirmId.trim() !== row.id) {
            formError.value = t('common.idMismatch');
            return;
          }
          row.status = type === 'enable' ? 'active' : 'disabled';
          addAudit(
            type === 'enable' ? 'account.enabled' : 'account.disabled',
            row.id,
            null,
            f.reason.trim(),
          );
          closeModal();
          notify('users.updated');
          return;
        }
        if (type === 'registration') {
          registration.value = !registration.value;
          addAudit('registration.updated');
          closeModal();
          notify('common.saved');
          return;
        }
        if (type === 'logout') {
          closeModal();
          go('login');
        }
      };
      const exportAudit = () => {
        if (!filteredRows.value.length) {
          notify('common.noExport');
          return;
        }
        const fields = [
          'action',
          'actorUserId',
          'subjectUserId',
          'projectId',
          'requestId',
          'createdAt',
        ];
        const quote = (value) =>
          '"' +
          String(value ?? '')
            .replace(/^[=+@-]/, ' $&')
            .replaceAll('"', '""') +
          '"';
        const csv =
          '\uFEFF' +
          [
            fields.map(quote).join(','),
            ...filteredRows.value.map((row) =>
              fields.map((key) => quote(row[key])).join(','),
            ),
          ].join('\r\n');
        const url = URL.createObjectURL(
          new Blob([csv], { type: 'text/csv;charset=utf-8' }),
        );
        const link = document.createElement('a');
        link.href = url;
        link.download = 'tasktips-demo-audit.csv';
        link.click();
        setTimeout(() => URL.revokeObjectURL(url), 1000);
        notify('audit.exported');
      };
      const authEmail = ref('');
      const authPassword = ref('');
      const authConfirm = ref('');
      const authError = ref('');
      const showPassword = ref(false);
      const registered = ref(false);
      const submitAuth = () => {
        authError.value = '';
        if (previewState.value === 'error') {
          authError.value = t('auth.failed');
          return;
        }
        if (page.value === 'register') {
          if (!validPassword(authPassword.value)) {
            authError.value = t('common.invalidPassword');
            return;
          }
          if (authPassword.value !== authConfirm.value) {
            authError.value = t('common.passwordMismatch');
            return;
          }
          if (!users.value.some((u) => u.email === authEmail.value))
            users.value.push({
              id: uid(users.value.length + 1),
              email: authEmail.value,
              role: 'user',
              status: 'pending',
              createdAt: '2026/09/19',
              lastLoginAt: null,
              tint: 'mint',
            });
          registered.value = true;
        } else {
          go('overview');
        }
        authPassword.value = '';
        authConfirm.value = '';
      };
      const palette = [
        {
          name: 'background',
          token: '--bg',
          light: '#F3F3F3',
          dark: '#202020',
        },
        {
          name: 'surface',
          token: '--surface',
          light: '#FFFFFF',
          dark: '#2B2B2B',
        },
        {
          name: 'accent',
          token: '--accent',
          light: '#0078D4',
          dark: '#4A9EFF',
        },
        { name: 'text', token: '--text', light: '#1B1B1B', dark: '#FFFFFF' },
        { name: 'success', token: '--ok', light: '#0F7B0F', dark: '#6CCB5F' },
        {
          name: 'danger',
          token: '--danger',
          light: '#C42B1C',
          dark: '#FF6B5E',
        },
      ];
      const routeChanged = () => {
        page.value = readPage();
        search.value = '';
        filter.value = 'all';
        pageNumber.value = 1;
        previewState.value = 'normal';
        authError.value = '';
        registered.value = false;
        authPassword.value = '';
        authConfirm.value = '';
        if (drawer.value) closeDrawer();
        if (modal.value) closeModal();
        document.title =
          t('brand.name') + ' · ' + t('nav.' + page.value) + ' · UI 设计稿';
        window.scrollTo(0, 0);
      };
      onMounted(() => {
        window.addEventListener('hashchange', routeChanged);
        systemTheme.addEventListener('change', applyTheme);
        refreshTimer = setInterval(() => {
          if (page.value === 'sync' && !document.hidden)
            updatedAt.value = new Date().toLocaleTimeString('zh-CN', {
              hour12: false,
            });
        }, 15000);
      });
      onBeforeUnmount(() => {
        window.removeEventListener('hashchange', routeChanged);
        systemTheme.removeEventListener('change', applyTheme);
        clearTimeout(toastTimer);
        clearInterval(refreshTimer);
      });
      return {
        t,
        page,
        isAuth,
        previewState,
        theme,
        themeMode,
        setTheme,
        toggleTheme,
        navigation,
        users,
        projects,
        devices,
        operations,
        mainMetrics,
        secondaryMetrics,
        healthItems,
        period,
        visibleTrends,
        trendStats,
        chartPoints,
        chartLine,
        chartArea,
        search,
        filter,
        setFilter,
        pageNumber,
        pageSize,
        filteredRows,
        pagedRows,
        tableColumns,
        number,
        shortId,
        updatedAt,
        refresh,
        toast,
        registration,
        drawerDialog,
        modalDialog,
        drawer,
        modal,
        showDrawer,
        closeDrawer,
        openModal,
        closeModal,
        dismissBackdrop,
        drawerTitle,
        drawerDetails,
        startRestore,
        goRelated,
        restoreStep,
        form,
        formError,
        submitModal,
        exportAudit,
        authEmail,
        authPassword,
        authConfirm,
        authError,
        showPassword,
        registered,
        submitAuth,
        go,
        palette,
      };
    },
  });
  app.component('app-icon', Icon);
  app.component('status-badge', Badge);
  app.use(
    createI18n({
      legacy: false,
      locale: 'zh-CN',
      fallbackLocale: 'zh-CN',
      messages,
    }),
  );
  app.mount('#app');
})();
