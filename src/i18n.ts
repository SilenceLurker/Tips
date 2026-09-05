import { readonly, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { createI18n } from "vue-i18n";
import { settingsApi } from "./api";
import enUS from "./locales/en-US";
import zhCN from "./locales/zh-CN";
import type { LocalePreference, SupportedLocale } from "./types";

const preference = ref<LocalePreference>("system");
const isMainView = new URLSearchParams(window.location.search).get("view") !== "sticky";
let confirmedPreference: LocalePreference = "system";
const activeLocale = ref<SupportedLocale>(resolveSystemLocale());
const saving = ref(false);
let initialization: Promise<void> | null = null;
let listening = false;
let preferenceGeneration = 0;
let writeGeneration = 0;
let writeQueue: Promise<void> = Promise.resolve();

export const i18n = createI18n({
  legacy: false,
  locale: activeLocale.value,
  fallbackLocale: "en-US",
  messages: {
    "zh-CN": zhCN,
    "en-US": enUS,
  },
});

function isLocalePreference(value: unknown): value is LocalePreference {
  return value === "system" || value === "zh-CN" || value === "en-US";
}

function resolveSystemLocale(): SupportedLocale {
  const languages = [...(navigator.languages ?? []), navigator.language];
  for (const language of languages) {
    const primary = language?.toLowerCase().split(/[-_]/, 1)[0];
    if (primary === "zh") return "zh-CN";
    if (primary === "en") return "en-US";
  }
  return "en-US";
}

function applyPreference(next: LocalePreference) {
  preference.value = next;
  activeLocale.value = next === "system" ? resolveSystemLocale() : next;
  i18n.global.locale.value = activeLocale.value;
  document.documentElement.lang = activeLocale.value;
}

applyPreference(preference.value);

export function initializeLocale(): Promise<void> {
  if (initialization) return initialization;

  initialization = (async () => {
    if (!listening) {
      try {
        await listen<LocalePreference>("locale-changed", ({ payload }) => {
          if (!isLocalePreference(payload)) return;
          confirmedPreference = payload;
          preferenceGeneration += 1;
          applyPreference(payload);
        });
        listening = true;
      } catch (cause) {
        console.error("Unable to listen for locale updates", cause);
      }

      window.addEventListener("languagechange", () => {
        if (preference.value !== "system") return;
        applyPreference("system");
        if (!isMainView) return;
        void setLocalePreference("system").catch((cause) => {
          console.error("Unable to refresh the native system locale", cause);
        });
      });
    }

    const loadGeneration = preferenceGeneration;
    try {
      const saved = await settingsApi.getLocale();
      if (loadGeneration === preferenceGeneration && isLocalePreference(saved)) {
        confirmedPreference = saved;
        applyPreference(saved);
      }
    } catch (cause) {
      console.error("Unable to load the saved locale", cause);
    }
  })();

  return initialization;
}

export async function setLocalePreference(next: LocalePreference): Promise<LocalePreference> {
  const generation = ++writeGeneration;
  preferenceGeneration += 1;
  saving.value = true;
  applyPreference(next);
  const previousWrite = writeQueue;
  let releaseWrite!: () => void;
  writeQueue = new Promise<void>((resolve) => {
    releaseWrite = resolve;
  });

  try {
    await previousWrite;
    const saved = await settingsApi.setLocale(next);
    confirmedPreference = saved;
    if (generation === writeGeneration) applyPreference(saved);
    return saved;
  } catch (cause) {
    if (generation === writeGeneration) applyPreference(confirmedPreference);
    throw cause;
  } finally {
    if (generation === writeGeneration) saving.value = false;
    releaseWrite();
  }
}

export function useLocale() {
  return {
    preference: readonly(preference),
    locale: readonly(activeLocale),
    saving: readonly(saving),
    setPreference: setLocalePreference,
  };
}
