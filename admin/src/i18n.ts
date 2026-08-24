import { createI18n } from 'vue-i18n';

const messages = {
  'zh-CN': {
    app: {
      name: 'TaskTips 管理后台',
      apiLive: 'API 在线',
      apiUnavailable: 'API 不可用',
      checking: '正在检查 API',
    },
    navigation: {
      overview: '总览',
      users: '用户',
      projects: '项目',
      devices: '设备',
      sync: '同步诊断',
      audit: '审计',
    },
    page: {
      noData: '暂无数据',
      overviewDescription: '服务连接后显示运行概况。',
      usersDescription: '服务连接后显示账号与邀请。',
      projectsDescription: '服务连接后显示项目元数据。',
      devicesDescription: '服务连接后显示设备状态。',
      syncDescription: '服务连接后显示同步诊断。',
      auditDescription: '服务连接后显示审计事件。',
    },
  },
  'en-US': {
    app: {
      name: 'TaskTips Admin',
      apiLive: 'API online',
      apiUnavailable: 'API unavailable',
      checking: 'Checking API',
    },
    navigation: {
      overview: 'Overview',
      users: 'Users',
      projects: 'Projects',
      devices: 'Devices',
      sync: 'Sync diagnostics',
      audit: 'Audit',
    },
    page: {
      noData: 'No data',
      overviewDescription:
        'Operational data appears after the service connects.',
      usersDescription:
        'Accounts and invitations appear after the service connects.',
      projectsDescription:
        'Project metadata appears after the service connects.',
      devicesDescription: 'Device status appears after the service connects.',
      syncDescription: 'Sync diagnostics appear after the service connects.',
      auditDescription: 'Audit events appear after the service connects.',
    },
  },
} as const;

export const i18n = createI18n({
  legacy: false,
  locale: 'zh-CN',
  fallbackLocale: 'en-US',
  messages,
});
