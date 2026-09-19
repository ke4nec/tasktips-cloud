<script setup lang="ts">
import { Lock, User } from '@element-plus/icons-vue';
import { ElAlert } from 'element-plus/es/components/alert/index.mjs';
import 'element-plus/es/components/alert/style/css.mjs';
import { ElButton } from 'element-plus/es/components/button/index.mjs';
import 'element-plus/es/components/button/style/css.mjs';
import { ElCard } from 'element-plus/es/components/card/index.mjs';
import 'element-plus/es/components/card/style/css.mjs';
import { ElForm, ElFormItem } from 'element-plus/es/components/form/index.mjs';
import 'element-plus/es/components/form/style/css.mjs';
import 'element-plus/es/components/form-item/style/css.mjs';
import { ElIcon } from 'element-plus/es/components/icon/index.mjs';
import 'element-plus/es/components/icon/style/css.mjs';
import { ElInput } from 'element-plus/es/components/input/index.mjs';
import 'element-plus/es/components/input/style/css.mjs';
import { ElSkeleton } from 'element-plus/es/components/skeleton/index.mjs';
import 'element-plus/es/components/skeleton/style/css.mjs';
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { RouterLink } from 'vue-router';

import { ApiError, apiClient } from '@/api/client';

const { t } = useI18n();
const email = ref('');
const password = ref('');
const confirm = ref('');
const loading = ref(false);
const checking = ref(true);
const registrationOpen = ref(false);
const error = ref<string>();
const succeeded = ref(false);
const passwordComplex = (value: string): boolean =>
  /[A-Za-z]/.test(value) && /[0-9]/.test(value);
const canSubmit = computed(
  () =>
    email.value.trim().length > 0 &&
    password.value.length >= 12 &&
    passwordComplex(password.value) &&
    confirm.value === password.value,
);

onMounted(async () => {
  try {
    const status = await apiClient.registrationStatus();
    registrationOpen.value = status.enabled;
  } catch (reason) {
    error.value =
      reason instanceof Error ? reason.message : t('page.loadFailed');
  } finally {
    checking.value = false;
  }
});

async function submit(): Promise<void> {
  if (!canSubmit.value || loading.value) return;
  loading.value = true;
  error.value = undefined;
  try {
    await apiClient.register(email.value.trim(), password.value);
    succeeded.value = true;
  } catch (reason) {
    error.value =
      reason instanceof ApiError ? reason.message : t('register.submitFailed');
  } finally {
    loading.value = false;
  }
}
</script>

<template>
  <main class="login-page">
    <el-card class="login-card" shadow="never">
      <h1>{{ t('app.name') }}</h1>
      <p>{{ t('register.subtitle') }}</p>
      <el-skeleton v-if="checking" :rows="3" animated />
      <el-alert
        v-else-if="!registrationOpen"
        :title="t('register.closed')"
        type="warning"
        show-icon
        :closable="false"
      />
      <el-alert
        v-else-if="succeeded"
        :title="t('register.succeeded')"
        type="success"
        show-icon
        :closable="false"
      />
      <el-form v-else @submit.prevent="submit">
        <el-form-item :label="t('auth.email')">
          <el-input v-model="email" type="email" autocomplete="username">
            <template #prefix
              ><el-icon><User /></el-icon
            ></template>
          </el-input>
        </el-form-item>
        <el-form-item :label="t('auth.password')">
          <el-input
            v-model="password"
            type="password"
            show-password
            autocomplete="new-password"
          >
            <template #prefix
              ><el-icon><Lock /></el-icon
            ></template>
          </el-input>
          <p class="field-hint">{{ t('register.passwordHint') }}</p>
        </el-form-item>
        <el-form-item :label="t('register.confirm')">
          <el-input
            v-model="confirm"
            type="password"
            show-password
            autocomplete="new-password"
          >
            <template #prefix
              ><el-icon><Lock /></el-icon
            ></template>
          </el-input>
        </el-form-item>
        <el-alert v-if="error" :title="error" type="error" show-icon />
        <el-button
          class="login-submit"
          type="primary"
          native-type="submit"
          :loading="loading"
          :disabled="!canSubmit"
        >
          {{ t('register.submit') }}
        </el-button>
      </el-form>
      <p class="auth-switch">
        <RouterLink to="/login">{{ t('register.backToLogin') }}</RouterLink>
      </p>
    </el-card>
  </main>
</template>
