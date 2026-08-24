<script setup lang="ts">
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
