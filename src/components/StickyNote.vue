<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useI18n } from "vue-i18n";
import { errorMessage, itemsApi } from "../api";
import { useTheme } from "../theme";
import { type GroupChangedEvent, type Item, type ItemColor, type TodoEntry } from "../types";

const props = defineProps<{ itemId: string }>();
const { t } = useI18n({ useScope: "global" });
const { theme, toggleTheme } = useTheme();
const colorValues: ItemColor[] = ["sun", "mint", "sky", "rose", "lilac", "paper"];

const item = ref<Item | null>(null);
const title = ref("");
const content = ref("");
const newTodoText = ref("");
const color = ref<ItemColor>("sun");
const pinned = ref(true);
const backgroundOpacity = ref(100);
const textOpacity = ref(100);
const appearancePanelOpen = ref(false);
const revealing = ref(false);
const ready = ref(false);
const saveState = ref<"idle" | "saving" | "saved" | "error">("idle");
const error = ref("");
const hidingWindow = ref(false);
const todoEntryDrafts = reactive<Record<string, string>>({});
const activeTodoEntryDraftId = ref<string | null>(null);
let saveTimer: number | undefined;
let opacitySaveTimer: number | undefined;
let stopListening: (() => void) | undefined;
let stopMcpListening: (() => void) | undefined;
let stopGroupsListening: (() => void) | undefined;
let mutationCount = 0;
let draftDirty = false;
let suppressDraftWatch = false;
let pendingExternalReload = false;
let opacitySaveInFlight = false;
let opacityDirty = false;

const shellClasses = computed(() => [
  "sticky-note",
  `color-${color.value}`,
  { "is-secret": item.value?.kind === "secret" },
  { "is-todo": item.value?.kind === "todo" },
]);
const shellStyle = computed<Record<string, string>>(() => ({
  "--sticky-background-opacity": String(backgroundOpacity.value / 100),
  "--sticky-text-opacity": String(textOpacity.value / 100),
}));

onMounted(async () => {
  document.documentElement.classList.add("sticky-document");
  stopListening = await listen<Item>("items-changed", ({ payload }) => {
    if (payload.id !== props.itemId) return;
    if (shouldDeferExternalReload()) {
      pendingExternalReload = true;
      return;
    }
    applyItem(payload);
  });
  stopMcpListening = await listen<string>("mcp-items-changed", async ({ payload }) => {
    if (payload !== props.itemId) return;
    if (shouldDeferExternalReload()) {
      pendingExternalReload = true;
      return;
    }
    await loadItem();
  });
  stopGroupsListening = await listen<GroupChangedEvent>("groups-changed", async ({ payload }) => {
    if (!payload.previousKey || item.value?.group !== payload.previousKey) return;
    if (shouldDeferExternalReload()) {
      pendingExternalReload = true;
      return;
    }
    await loadItem();
  });
  await loadItem();
  window.addEventListener("keydown", handleShortcut);
});

onBeforeUnmount(() => {
  if (saveTimer) window.clearTimeout(saveTimer);
  if (opacitySaveTimer) window.clearTimeout(opacitySaveTimer);
  stopListening?.();
  stopMcpListening?.();
  stopGroupsListening?.();
  window.removeEventListener("keydown", handleShortcut);
});

watch([title, content], () => {
  if (!ready.value || suppressDraftWatch) return;
  draftDirty = true;
  scheduleSave();
}, { flush: "sync" });

async function loadItem(preserveDraft = false): Promise<boolean> {
  try {
    applyItem(await itemsApi.get(props.itemId), preserveDraft);
    ready.value = true;
    return true;
  } catch (cause) {
    error.value = errorMessage(cause, t("errors.unknown"));
    return false;
  }
}

function applyItem(next: Item, preserveDraft = false) {
  if (item.value && next.revision < item.value.revision) return;

  const preserveOpacity = opacityDirty || opacitySaveInFlight || opacitySaveTimer !== undefined;
  item.value = next;
  suppressDraftWatch = true;
  if (!preserveDraft) {
    title.value = next.title;
    content.value = next.content;
    color.value = next.color;
    draftDirty = false;
  }
  pinned.value = next.window?.alwaysOnTop ?? true;
  if (!preserveOpacity) {
    backgroundOpacity.value = Math.round((next.window?.backgroundOpacity ?? 1) * 100);
    textOpacity.value = Math.round((next.window?.textOpacity ?? 1) * 100);
  }
  const liveTodoIds = new Set(next.todoEntries.map((entry) => entry.id));
  for (const draftId of Object.keys(todoEntryDrafts)) {
    if (!liveTodoIds.has(draftId)) delete todoEntryDrafts[draftId];
  }
  suppressDraftWatch = false;
}

function shouldDeferExternalReload(): boolean {
  return (
    mutationCount > 0 ||
    draftDirty ||
    saveTimer !== undefined ||
    activeTodoEntryDraftId.value !== null ||
    opacityDirty ||
    opacitySaveTimer !== undefined
  );
}

async function refreshPendingExternalIfSafe() {
  if (!pendingExternalReload || shouldDeferExternalReload()) return;
  pendingExternalReload = false;
  if (!(await loadItem())) pendingExternalReload = true;
}

function markDraftCleanIfItMatchesItem() {
  if (!item.value) return;
  if (
    item.value.title === title.value &&
    item.value.content === content.value &&
    item.value.color === color.value
  ) {
    draftDirty = false;
  }
}

function scheduleSave() {
  saveState.value = "idle";
  if (saveTimer) window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    saveTimer = undefined;
    void saveNow();
  }, 550);
}

async function chooseColor(next: ItemColor) {
  if (color.value === next) return;
  color.value = next;
  draftDirty = true;
  await saveNow();
}

async function saveNow() {
  if (saveTimer) {
    window.clearTimeout(saveTimer);
    saveTimer = undefined;
  }
  markDraftCleanIfItMatchesItem();
  if (mutationCount > 0) {
    if (draftDirty) scheduleSave();
    return;
  }
  if (
    !item.value ||
    (!title.value.trim() &&
      !content.value.trim() &&
      item.value.todoEntries.length === 0)
  ) {
    if (item.value) {
      saveState.value = "error";
      error.value = t("errors.emptyItem");
    }
    await refreshPendingExternalIfSafe();
    return;
  }

  if (pendingExternalReload && draftDirty) {
    if (!(await loadItem(true))) {
      pendingExternalReload = true;
      return;
    }
    pendingExternalReload = false;
  }
  if (
    item.value.title === title.value &&
    item.value.content === content.value &&
    item.value.color === color.value
  ) {
    draftDirty = false;
    await refreshPendingExternalIfSafe();
    return;
  }

  const submitted = {
    title: title.value,
    content: content.value,
    color: color.value,
  };
  mutationCount += 1;
  saveState.value = "saving";
  try {
    const updated = await itemsApi.update({
      id: item.value.id,
      expectedRevision: item.value.revision,
      group: item.value.group,
      title: submitted.title,
      content: submitted.content,
      color: submitted.color,
      tags: item.value.tags,
      dueAt: item.value.dueAt,
      expiresAt: item.value.expiresAt,
    });
    const changedDuringSave =
      title.value !== submitted.title ||
      content.value !== submitted.content ||
      color.value !== submitted.color;
    applyItem(updated, changedDuringSave);
    draftDirty = changedDuringSave;
    if (changedDuringSave && saveTimer === undefined) scheduleSave();
    saveState.value = "saved";
    window.setTimeout(() => {
      if (saveState.value === "saved") saveState.value = "idle";
    }, 1200);
  } catch (cause) {
    saveState.value = "error";
    error.value = errorMessage(cause, t("errors.unknown"));
    if (error.value.includes("changed elsewhere")) {
      await loadItem(true);
      pendingExternalReload = false;
      draftDirty = true;
      error.value = t("errors.conflictPreserved");
    }
  } finally {
    mutationCount -= 1;
    await refreshPendingExternalIfSafe();
  }
}

function scheduleOpacitySave() {
  saveState.value = "idle";
  opacityDirty = true;
  if (opacitySaveTimer) window.clearTimeout(opacitySaveTimer);
  opacitySaveTimer = window.setTimeout(() => {
    opacitySaveTimer = undefined;
    void saveOpacityNow();
  }, 180);
}

async function saveOpacityNow() {
  if (!item.value) return;
  if (opacitySaveTimer) {
    window.clearTimeout(opacitySaveTimer);
    opacitySaveTimer = undefined;
  }
  if (opacitySaveInFlight) {
    opacityDirty = true;
    return;
  }

  const current = item.value.window;
  if (
    current &&
    Math.round(current.backgroundOpacity * 100) === backgroundOpacity.value &&
    Math.round(current.textOpacity * 100) === textOpacity.value
  ) {
    opacityDirty = false;
    await refreshPendingExternalIfSafe();
    return;
  }

  const submittedBackgroundOpacity = backgroundOpacity.value;
  const submittedTextOpacity = textOpacity.value;
  opacityDirty = false;
  opacitySaveInFlight = true;
  let shouldRetryImmediately = false;
  mutationCount += 1;
  saveState.value = "saving";
  try {
    const state = await itemsApi.setStickyOpacity(
      item.value.id,
      submittedBackgroundOpacity / 100,
      submittedTextOpacity / 100,
    );
    item.value = { ...item.value, window: state };
    const changedDuringSave =
      backgroundOpacity.value !== submittedBackgroundOpacity ||
      textOpacity.value !== submittedTextOpacity;
    if (!changedDuringSave) {
      backgroundOpacity.value = Math.round(state.backgroundOpacity * 100);
      textOpacity.value = Math.round(state.textOpacity * 100);
    } else {
      opacityDirty = true;
      shouldRetryImmediately = true;
    }
    markDraftCleanIfItMatchesItem();
    saveState.value = "saved";
    window.setTimeout(() => {
      if (saveState.value === "saved") saveState.value = "idle";
    }, 1200);
  } catch (cause) {
    saveState.value = "error";
    error.value = errorMessage(cause, t("errors.unknown"));
    opacityDirty = true;
  } finally {
    mutationCount -= 1;
    opacitySaveInFlight = false;
    if (shouldRetryImmediately && opacityDirty && opacitySaveTimer === undefined) {
      opacitySaveTimer = window.setTimeout(() => {
        opacitySaveTimer = undefined;
        void saveOpacityNow();
      }, 0);
    }
    await refreshPendingExternalIfSafe();
  }
}

async function togglePin() {
  if (!item.value) return;
  try {
    const state = await itemsApi.setStickyPin(item.value.id, !pinned.value);
    pinned.value = state.alwaysOnTop;
  } catch (cause) {
    error.value = errorMessage(cause, t("errors.unknown"));
  }
}

async function toggleTodo() {
  if (!item.value || item.value.kind !== "todo") return;
  await runTodoMutation((current) =>
    itemsApi.setStatus(current, current.status === "completed" ? "active" : "completed"),
  );
}

function todoMutationIsBusy(): boolean {
  return mutationCount > 0 || draftDirty || saveTimer !== undefined;
}

async function runTodoMutation(action: (current: Item) => Promise<Item>) {
  if (!item.value || item.value.kind !== "todo" || mutationCount > 0) return;
  await saveNow();
  markDraftCleanIfItMatchesItem();
  if (!item.value || todoMutationIsBusy()) return;

  mutationCount += 1;
  saveState.value = "saving";
  error.value = "";
  const draftAtMutationStart = {
    title: title.value,
    content: content.value,
    color: color.value,
  };
  try {
    const updated = await action(item.value);
    applyItem(updated, draftDirty);
    saveState.value = "saved";
    window.setTimeout(() => {
      if (saveState.value === "saved") saveState.value = "idle";
    }, 1200);
  } catch (cause) {
    saveState.value = "error";
    error.value = errorMessage(cause, t("errors.unknown"));
    const changedDuringMutation =
      draftDirty ||
      title.value !== draftAtMutationStart.title ||
      content.value !== draftAtMutationStart.content ||
      color.value !== draftAtMutationStart.color;
    await loadItem(changedDuringMutation);
    if (changedDuringMutation) {
      draftDirty = true;
    }
  } finally {
    mutationCount -= 1;
    await refreshPendingExternalIfSafe();
  }
}

async function toggleTodoEntry(entry: TodoEntry) {
  await runTodoMutation((current) =>
    itemsApi.setTodoEntryCompleted(current, entry, !entry.isCompleted),
  );
}

function todoEntryDraftValue(entry: TodoEntry): string {
  return todoEntryDrafts[entry.id] ?? entry.text;
}

function updateTodoEntryDraft(entry: TodoEntry, event: Event) {
  todoEntryDrafts[entry.id] = (event.target as HTMLInputElement).value;
}

async function renameTodoEntry(entry: TodoEntry) {
  activeTodoEntryDraftId.value = null;
  const normalized = todoEntryDraftValue(entry).trim();
  if (!normalized) {
    delete todoEntryDrafts[entry.id];
    await refreshPendingExternalIfSafe();
    return;
  }
  if (normalized === entry.text) {
    delete todoEntryDrafts[entry.id];
    await refreshPendingExternalIfSafe();
    return;
  }
  await runTodoMutation((current) =>
    itemsApi.updateTodoEntry(current, entry, normalized),
  );
  if (saveState.value !== "error") delete todoEntryDrafts[entry.id];
}

function beginTodoEntryDraft(entry: TodoEntry) {
  activeTodoEntryDraftId.value = entry.id;
  if (!(entry.id in todoEntryDrafts)) todoEntryDrafts[entry.id] = entry.text;
}

async function cancelTodoEntryDraft(entry: TodoEntry, event: Event) {
  const input = event.target as HTMLInputElement;
  delete todoEntryDrafts[entry.id];
  activeTodoEntryDraftId.value = null;
  input.blur();
  await refreshPendingExternalIfSafe();
}

async function finishTodoEntryDraft(entry: TodoEntry) {
  if (activeTodoEntryDraftId.value !== entry.id) return;
  await renameTodoEntry(entry);
}

async function deleteTodoEntry(entry: TodoEntry) {
  await runTodoMutation((current) => itemsApi.deleteTodoEntry(current, entry));
}

async function addTodoEntry() {
  const text = newTodoText.value.trim();
  if (!text) return;
  await runTodoMutation((current) => itemsApi.addTodoEntry(current, text));
  if (saveState.value !== "error") newTodoText.value = "";
}

const todoProgress = computed(() => {
  const entries = item.value?.todoEntries ?? [];
  return {
    completed: entries.filter((entry) => entry.isCompleted).length,
    total: entries.length,
  };
});

async function hideWindow() {
  if (!item.value || hidingWindow.value) return;
  hidingWindow.value = true;
  error.value = "";
  try {
    if (document.activeElement instanceof HTMLElement) {
      document.activeElement.blur();
    }
    await nextTick();

    if (saveTimer) {
      window.clearTimeout(saveTimer);
      saveTimer = undefined;
    }
    if (opacitySaveTimer) {
      window.clearTimeout(opacitySaveTimer);
      opacitySaveTimer = undefined;
      opacityDirty = true;
    }

    const deadline = Date.now() + 10_000;
    while (true) {
      if (mutationCount === 0) {
        markDraftCleanIfItMatchesItem();
        if (draftDirty) await saveNow();
        if (opacityDirty) await saveOpacityNow();
      }

      if (
        mutationCount === 0 &&
        !draftDirty &&
        !opacityDirty &&
        saveTimer === undefined &&
        opacitySaveTimer === undefined &&
        !opacitySaveInFlight
      ) {
        break;
      }
      if (saveState.value === "error") {
        throw new Error(error.value || t("errors.saveFailed"));
      }
      if (Date.now() >= deadline) {
        throw new Error(t("errors.saveTimeout"));
      }
      await new Promise<void>((resolve) => window.setTimeout(resolve, 25));
    }

    await itemsApi.hideSticky(item.value.id);
  } catch (cause) {
    saveState.value = "error";
    error.value = errorMessage(cause, t("errors.unknown"));
  } finally {
    hidingWindow.value = false;
  }
}

async function minimizeWindow() {
  await getCurrentWindow().minimize();
}

function handleShortcut(event: KeyboardEvent) {
  if (event.key === "Escape" && appearancePanelOpen.value) {
    appearancePanelOpen.value = false;
    return;
  }
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "w") {
    event.preventDefault();
    void hideWindow();
  }
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
    event.preventDefault();
    void saveNow();
  }
}

async function switchTheme() {
  try {
    await toggleTheme();
  } catch (cause) {
    error.value = errorMessage(cause, t("errors.unknown"));
  }
}
</script>

<template>
  <main
    :class="[shellClasses, { 'is-hiding': hidingWindow }]"
    :style="shellStyle"
    :inert="hidingWindow"
    :aria-busy="hidingWindow"
  >
    <header class="sticky-titlebar" data-tauri-drag-region>
      <div class="sticky-grip" data-tauri-drag-region>
        <span></span><span></span><span></span>
      </div>
      <div class="sticky-window-actions">
        <button
          :class="{ active: pinned }"
          :title="pinned ? t('common.actions.unpin') : t('common.actions.pin')"
          :aria-label="t('sticky.actions.togglePin')"
          @click="togglePin"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="m9 3 6 0-.8 5 3.3 3.2v1.3H13v6l-1 2-1-2v-6H6.5v-1.3L9.8 8 9 3Z" />
          </svg>
        </button>
        <button
          :class="{ active: appearancePanelOpen }"
          :title="t('appearance.title')"
          :aria-label="t('appearance.title')"
          @click="appearancePanelOpen = !appearancePanelOpen"
        >
          <span class="opacity-toggle-icon" aria-hidden="true">&#9680;</span>
        </button>
        <button :title="t('common.actions.minimize')" :aria-label="t('common.actions.minimize')" @click="minimizeWindow">&#8722;</button>
        <button :title="t('common.actions.hideSticky')" :aria-label="t('common.actions.hideSticky')" @click="hideWindow">×</button>
      </div>
    </header>

    <section
      v-if="appearancePanelOpen"
      class="opacity-panel"
      :aria-label="t('appearance.windowLabel')"
      @pointerdown.stop
      @click.stop
    >
      <header>
        <strong>{{ t("appearance.heading") }}</strong>
        <button :aria-label="t('appearance.close')" @click="appearancePanelOpen = false">×</button>
      </header>
      <div class="theme-control">
        <span>{{ t("appearance.theme") }}</span>
        <button
          :class="{ active: theme === 'black_gold' }"
          :aria-pressed="theme === 'black_gold'"
          @click="switchTheme"
        >
          <span class="theme-chip" aria-hidden="true"></span>
          {{ theme === "black_gold" ? t("themes.blackGold") : t("themes.light") }}
        </button>
      </div>
      <div class="sticky-color-control">
        <span>{{ t("appearance.backgroundColor") }}</span>
        <div class="sticky-color-picker">
          <button
            v-for="option in colorValues"
            :key="option"
            :class="[`swatch-${option}`, { active: color === option }]"
            :title="t(`colors.${option}`)"
            :aria-label="t('appearance.backgroundColorLabel', { color: t(`colors.${option}`) })"
            :aria-pressed="color === option"
            @click="chooseColor(option)"
          >
            <span></span>
          </button>
        </div>
      </div>
      <div class="appearance-divider"></div>
      <label>
        <span>{{ t("appearance.backgroundOpacity") }}</span>
        <output>{{ backgroundOpacity }}%</output>
        <input
          v-model.number="backgroundOpacity"
          type="range"
          min="10"
          max="100"
          step="5"
          @input="scheduleOpacitySave"
        />
      </label>
      <label>
        <span>{{ t("appearance.textOpacity") }}</span>
        <output>{{ textOpacity }}%</output>
        <input
          v-model.number="textOpacity"
          type="range"
          min="20"
          max="100"
          step="5"
          @input="scheduleOpacitySave"
        />
      </label>
      <small>{{ t("appearance.autoSaveHint") }}</small>
    </section>

    <section v-if="item" class="sticky-content">
      <div class="sticky-meta">
        <button
          v-if="item.kind === 'todo'"
          class="sticky-check"
          :class="{ checked: item.status === 'completed' }"
          :aria-label="item.status === 'completed' ? t('todo.restoreTask') : t('todo.completeTask')"
          @click="toggleTodo"
        >&#10003;</button>
        <span class="sticky-kind">
          {{
            item.kind === "todo"
              ? t("items.kinds.todo")
              : item.kind === "secret"
                ? t("items.kinds.encryptedSecret")
                : t("items.kinds.note")
          }}
        </span>
        <span class="sticky-group"># {{ item.group }}</span>
        <span v-if="item.kind === 'todo'" class="sticky-todo-progress">
          {{ todoProgress.completed }}/{{ todoProgress.total }}
        </span>
        <span v-if="item.kind === 'secret'" class="mcp-denied">{{ t("items.mcpDenied") }}</span>
      </div>

      <input
        v-model="title"
        class="sticky-title-input"
        maxlength="160"
        :aria-label="t('items.title.label')"
        :placeholder="t('items.title.untitled')"
      />

      <section v-if="item.kind === 'todo'" class="sticky-todo-list" :aria-label="t('todo.listLabel')">
        <div v-if="item.todoEntries.length" class="sticky-todo-items">
          <div v-for="entry in item.todoEntries" :key="entry.id" class="sticky-todo-row">
            <button
              type="button"
              class="sticky-entry-check"
              :class="{ checked: entry.isCompleted }"
              :title="entry.isCompleted ? t('todo.restoreEntry') : t('todo.completeEntry')"
              @click="toggleTodoEntry(entry)"
            >
              &#10003;
            </button>
            <input
              :value="todoEntryDraftValue(entry)"
              class="sticky-entry-text"
              :class="{ completed: entry.isCompleted }"
              maxlength="500"
              spellcheck="false"
              :aria-label="t('todo.entryLabel')"
              @focus="beginTodoEntryDraft(entry)"
              @input="updateTodoEntryDraft(entry, $event)"
              @change="renameTodoEntry(entry)"
              @blur="finishTodoEntryDraft(entry)"
              @keydown.esc.prevent="cancelTodoEntryDraft(entry, $event)"
            />
            <button
              type="button"
              class="sticky-entry-delete"
              :title="t('todo.deleteEntry')"
              :aria-label="t('todo.deleteEntry')"
              @click="deleteTodoEntry(entry)"
            >
              ×
            </button>
          </div>
        </div>
        <p v-else class="sticky-todo-empty">{{ t("todo.empty") }}</p>
        <form class="sticky-todo-add" @submit.prevent="addTodoEntry">
          <input
            v-model="newTodoText"
            maxlength="500"
            :placeholder="t('todo.addPlaceholder')"
            :aria-label="t('todo.add')"
          />
          <button type="submit" :disabled="!newTodoText.trim()" :title="t('todo.add')">＋</button>
        </form>
      </section>

      <template v-if="item.kind === 'secret' && !revealing">
        <button class="secret-cover" @click="revealing = true">
          <span>◇</span>
          <strong>{{ t("secret.hidden") }}</strong>
          <small>{{ t("secret.revealHint") }}</small>
        </button>
      </template>
      <textarea
        v-else
        v-model="content"
        class="sticky-body-input"
        :aria-label="t('items.content.label')"
        :placeholder="t('items.content.placeholder')"
        spellcheck="false"
      ></textarea>

      <button
        v-if="item.kind === 'secret' && revealing"
        class="hide-secret"
        @click="revealing = false"
      >
        {{ t("secret.hide") }}
      </button>
    </section>

    <section v-else-if="error" class="sticky-error">{{ error }}</section>
    <section v-else class="sticky-loading">{{ t("sticky.loading") }}</section>

    <footer v-if="item" class="sticky-footer">
      <span class="save-state" :class="saveState">
        {{
          saveState === "saving"
            ? t("sticky.saveState.saving")
            : saveState === "saved"
              ? t("sticky.saveState.saved")
              : saveState === "error"
                ? t("sticky.saveState.error")
                : pinned
                  ? t("sticky.saveState.pinned")
                  : t("sticky.saveState.normal")
        }}
      </span>
    </footer>
  </main>
</template>
