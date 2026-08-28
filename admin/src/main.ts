import './styles/main.css';

import { createPinia } from 'pinia';
import { createApp } from 'vue';

import App from './App.vue';
import { i18n } from './i18n';
import { router } from './router';

document.documentElement.lang = i18n.global.locale.value;
document.title = i18n.global.t('app.name');

createApp(App).use(createPinia()).use(router).use(i18n).mount('#app');
