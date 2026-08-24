import { createRouter, createWebHistory } from 'vue-router';

import EmptySection from './views/EmptySection.vue';

export const router = createRouter({
  history: createWebHistory('/admin/'),
  routes: [
    {
      path: '/',
      name: 'overview',
      component: EmptySection,
      props: {
        titleKey: 'navigation.overview',
        descriptionKey: 'page.overviewDescription',
      },
    },
    {
      path: '/users',
      name: 'users',
      component: EmptySection,
      props: {
        titleKey: 'navigation.users',
        descriptionKey: 'page.usersDescription',
      },
    },
    {
      path: '/projects',
      name: 'projects',
      component: EmptySection,
      props: {
        titleKey: 'navigation.projects',
        descriptionKey: 'page.projectsDescription',
      },
    },
    {
      path: '/devices',
      name: 'devices',
      component: EmptySection,
      props: {
        titleKey: 'navigation.devices',
        descriptionKey: 'page.devicesDescription',
      },
    },
    {
      path: '/sync-attempts',
      name: 'sync-attempts',
      component: EmptySection,
      props: {
        titleKey: 'navigation.sync',
        descriptionKey: 'page.syncDescription',
      },
    },
    {
      path: '/audit',
      name: 'audit',
      component: EmptySection,
      props: {
        titleKey: 'navigation.audit',
        descriptionKey: 'page.auditDescription',
      },
    },
  ],
});
