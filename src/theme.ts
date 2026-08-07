import { readonly, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { settingsApi } from "./api";
import type { ThemeMode } from "./types";

const theme = ref<ThemeMode>("light");
let initialization: Promise<void> | null = null;
let listening = false;

function applyTheme(next: ThemeMode) {
  theme.value = next;
  document.documentElement.dataset.theme = next;
  document.documentElement.style.colorScheme = next === "black_gold" ? "dark" : "light";
}

applyTheme(theme.value);

export function initializeTheme(): Promise<void> {
  if (initialization) return initialization;

  initialization = (async () => {
    if (!listening) {
      try {
        await listen<ThemeMode>("theme-changed", ({ payload }) => {
          applyTheme(payload);
        });
        listening = true;
      } catch (cause) {
        console.error("Unable to listen for theme updates", cause);
      }
    }

    try {
      applyTheme(await settingsApi.getTheme());
    } catch (cause) {
      console.error("Unable to load the saved theme", cause);
    }
  })();

  return initialization;
}

export async function setTheme(next: ThemeMode): Promise<ThemeMode> {
  const previous = theme.value;
  applyTheme(next);
  try {
    const saved = await settingsApi.setTheme(next);
    applyTheme(saved);
    return saved;
  } catch (cause) {
    applyTheme(previous);
    throw cause;
  }
}

export function useTheme() {
  return {
    theme: readonly(theme),
    setTheme,
    toggleTheme: () => setTheme(theme.value === "black_gold" ? "light" : "black_gold"),
  };
}
