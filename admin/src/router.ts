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
      path: '/',
      name: 'overview',
      component: () => import('./views/OverviewView.vue'),
    },
    {
      path: '/users',
      name: 'users',
      component: () => import('./views/AdminDataView.vue'),
      props: { section: 'users' },
    },
    {
      path: '/projects',
      name: 'projects',
      component: () => import('./views/AdminDataView.vue'),
      props: { section: 'projects' },
    },
    {
      path: '/devices',
      name: 'devices',
      component: () => import('./views/AdminDataView.vue'),
      props: { section: 'devices' },
    },
    {
      path: '/sync-attempts',
      name: 'sync-attempts',
      component: () => import('./views/AdminDataView.vue'),
      props: { section: 'sync' },
    },
    {
      path: '/audit',
      name: 'audit',
      component: () => import('./views/AdminDataView.vue'),
      props: { section: 'audit' },
    },
  ],
});
