<script setup lang="ts">
import { ElEmpty } from 'element-plus/es/components/empty/index.mjs';
import 'element-plus/es/components/empty/style/css.mjs';
import { useI18n } from 'vue-i18n';

defineProps<{
  titleKey: string;
  descriptionKey: string;
}>();

const { t } = useI18n();
</script>

<template>
  <section class="empty-section" aria-labelledby="page-title">
    <header>
      <h1 id="page-title">{{ t(titleKey) }}</h1>
      <p>{{ t(descriptionKey) }}</p>
    </header>
    <el-empty :description="t('page.noData')" />
  </section>
</template>
