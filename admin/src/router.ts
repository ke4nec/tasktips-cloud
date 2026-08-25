import { createRouter, createWebHistory } from 'vue-router';

import LoginView from './views/LoginView.vue';
import OverviewView from './views/OverviewView.vue';
import AdminDataView from './views/AdminDataView.vue';

export const router = createRouter({
  history: createWebHistory('/admin/'),
  routes: [
    { path: '/login', name: 'login', component: LoginView },
    {
      path: '/',
      name: 'overview',
      component: OverviewView,
    },
    {
      path: '/users',
      name: 'users',
      component: AdminDataView,
      props: { section: 'users' },
    },
    {
      path: '/projects',
      name: 'projects',
      component: AdminDataView,
      props: { section: 'projects' },
    },
    {
      path: '/devices',
      name: 'devices',
      component: AdminDataView,
      props: { section: 'devices' },
    },
    {
      path: '/sync-attempts',
      name: 'sync-attempts',
      component: AdminDataView,
      props: { section: 'sync' },
    },
    {
      path: '/audit',
      name: 'audit',
      component: AdminDataView,
      props: { section: 'audit' },
    },
  ],
});
