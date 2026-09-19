import { createRouter, createWebHistory } from 'vue-router';

export const router = createRouter({
  history: createWebHistory('/admin/'),
  routes: [
    {
      path: '/login',
      name: 'login',
      component: () => import('./views/LoginView.vue'),
    },
    {
      path: '/register',
      name: 'register',
      component: () => import('./views/RegisterView.vue'),
    },
    {
      path: '/',
      name: 'overview',
      component: () => import('./views/OverviewView.vue'),
    },
    {
      path: '/users',
      name: 'users',
      component: () => import('./views/UsersView.vue'),
    },
    {
      path: '/projects',
      name: 'projects',
      component: () => import('./views/ProjectsView.vue'),
    },
    {
      path: '/devices',
      name: 'devices',
      component: () => import('./views/DevicesView.vue'),
    },
    {
      path: '/sync-attempts',
      name: 'sync-attempts',
      component: () => import('./views/SyncView.vue'),
    },
    {
      path: '/audit',
      name: 'audit',
      component: () => import('./views/AuditView.vue'),
    },
    {
      path: '/settings',
      name: 'settings',
      component: () => import('./views/SettingsView.vue'),
    },
  ],
});
