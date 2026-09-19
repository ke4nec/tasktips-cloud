/** Module-level toast state so any view can post a short confirmation. */
import { ref } from 'vue';

const message = ref('');
let timer: ReturnType<typeof setTimeout> | undefined;

export function useToast() {
  function show(text: string): void {
    if (timer) clearTimeout(timer);
    message.value = text;
    timer = setTimeout(() => {
      message.value = '';
    }, 3600);
  }
  return { message, show };
}
