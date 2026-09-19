<script setup lang="ts">
import { useI18n } from 'vue-i18n';

import AppIcon from './AppIcon.vue';

withDefaults(
  defineProps<{
    state: 'loading' | 'empty' | 'error';
    requestId?: string;
    compact?: boolean;
  }>(),
  { requestId: undefined, compact: false },
);
const { t } = useI18n();
defineEmits<{ retry: [] }>();
</script>

<template>
  <section
    class="panel state-panel"
    :class="{ 'state-compact': compact }"
    :aria-busy="state === 'loading'"
  >
    <template v-if="state === 'loading'">
      <div class="skeleton skeleton-title"></div>
      <div v-for="index in 6" :key="index" class="skeleton"></div>
      <span class="sr-only">{{ t('states.loading') }}</span>
    </template>
    <template v-else>
      <div class="empty-icon">
        <AppIcon :name="state === 'error' ? 'alert' : 'inbox'" />
      </div>
      <h2>{{ t(`states.${state}Title`) }}</h2>
      <p>{{ t(`states.${state}Body`) }}</p>
      <code v-if="state === 'error' && requestId">{{ requestId }}</code>
      <button v-if="state === 'error'" class="button" @click="$emit('retry')">
        {{ t('common.retry') }}
      </button>
    </template>
  </section>
</template>
