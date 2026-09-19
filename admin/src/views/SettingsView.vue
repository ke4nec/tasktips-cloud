<script setup lang="ts">
import { ElAlert } from 'element-plus/es/components/alert/index.mjs';
import 'element-plus/es/components/alert/style/css.mjs';
import { ElCard } from 'element-plus/es/components/card/index.mjs';
import 'element-plus/es/components/card/style/css.mjs';
import { ElSkeleton } from 'element-plus/es/components/skeleton/index.mjs';
import 'element-plus/es/components/skeleton/style/css.mjs';
import { ElSwitch } from 'element-plus/es/components/switch/index.mjs';
import 'element-plus/es/components/switch/style/css.mjs';
import { onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';

import { apiClient } from '@/api/client';

const { t } = useI18n();
const loading = ref(true);
const saving = ref(false);
const enabled = ref(false);
const error = ref<string>();
const message = ref<string>();

async function load(): Promise<void> {
  loading.value = true;
  error.value = undefined;
  try {
    const settings = await apiClient.getRegistrationSettings();
    enabled.value = settings.enabled;
  } catch (reason) {
    error.value =
      reason instanceof Error ? reason.message : t('page.loadFailed');
  } finally {
    loading.value = false;
  }
}

async function save(value: boolean): Promise<void> {
  saving.value = true;
  error.value = undefined;
  message.value = undefined;
  try {
    const settings = await apiClient.updateRegistrationSettings(value);
    enabled.value = settings.enabled;
    message.value = t('settings.saved');
  } catch (reason) {
    error.value =
      reason instanceof Error ? reason.message : t('page.loadFailed');
    await load();
  } finally {
    saving.value = false;
  }
}

onMounted(load);
</script>

<template>
  <section class="data-section">
    <header class="page-header">
      <div>
        <h1>{{ t('navigation.settings') }}</h1>
        <p>{{ t('settings.description') }}</p>
      </div>
    </header>
    <el-alert v-if="error" :title="error" type="error" show-icon />
    <el-alert v-if="message" :title="message" type="success" show-icon />
    <el-skeleton v-if="loading" :rows="3" animated />
    <el-card v-else shadow="never">
      <div class="setting-row">
        <div>
          <strong>{{ t('settings.registration') }}</strong>
          <p>{{ t('settings.registrationHint') }}</p>
        </div>
        <el-switch
          :model-value="enabled"
          :loading="saving"
          @change="save(Boolean($event))"
        />
      </div>
    </el-card>
  </section>
</template>
