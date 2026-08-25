<script setup lang="ts">
import { Lock, User } from '@element-plus/icons-vue';
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';

import { useAuthStore } from '@/stores/auth';

const { t } = useI18n();
const router = useRouter();
const auth = useAuthStore();
const email = ref('');
const password = ref('');
const canSubmit = computed(() => email.value.trim().length > 0 && password.value.length > 0);

async function submit(): Promise<void> {
  if (!canSubmit.value) return;
  if (await auth.login(email.value, password.value)) await router.push('/');
}
</script>

<template>
  <main class="login-page">
    <el-card class="login-card" shadow="never">
      <h1>{{ t('app.name') }}</h1>
      <p>{{ t('auth.subtitle') }}</p>
      <el-form @submit.prevent="submit">
        <el-form-item :label="t('auth.email')">
          <el-input v-model="email" type="email" autocomplete="username">
            <template #prefix><el-icon><User /></el-icon></template>
          </el-input>
        </el-form-item>
        <el-form-item :label="t('auth.password')">
          <el-input v-model="password" type="password" show-password autocomplete="current-password">
            <template #prefix><el-icon><Lock /></el-icon></template>
          </el-input>
        </el-form-item>
        <el-alert v-if="auth.error" :title="auth.error" type="error" show-icon />
        <el-button class="login-submit" type="primary" native-type="submit" :loading="auth.loading" :disabled="!canSubmit">
          {{ t('auth.signIn') }}
        </el-button>
      </el-form>
    </el-card>
  </main>
</template>
