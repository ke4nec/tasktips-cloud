<script setup lang="ts">
/** Thin wrapper over the native <dialog> element. showModal() provides focus
 * trapping and Escape handling; closing returns focus to the opener. */
import { nextTick, ref, watch } from 'vue';

const props = defineProps<{
  open: boolean;
  variant?: 'modal' | 'drawer';
}>();
const emit = defineEmits<{ close: [] }>();

const dialog = ref<HTMLDialogElement>();

watch(
  () => props.open,
  async (open) => {
    await nextTick();
    if (open && dialog.value && !dialog.value.open) dialog.value.showModal();
    if (!open && dialog.value?.open) dialog.value.close();
  },
  { immediate: true },
);

function onClick(event: MouseEvent): void {
  // Close only for clicks on the backdrop itself, not on dialog content.
  if (event.target !== dialog.value) return;
  const rect = dialog.value!.getBoundingClientRect();
  if (
    event.clientX < rect.left ||
    event.clientX > rect.right ||
    event.clientY < rect.top ||
    event.clientY > rect.bottom
  ) {
    emit('close');
  }
}
</script>

<template>
  <dialog
    ref="dialog"
    :class="variant ?? 'modal'"
    @close="emit('close')"
    @click="onClick"
  >
    <slot />
  </dialog>
</template>
