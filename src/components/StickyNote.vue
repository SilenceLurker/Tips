<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { errorMessage, itemsApi } from "../api";
import { useTheme } from "../theme";
import { colorOptions, type Item, type ItemColor, type TodoEntry } from "../types";

const props = defineProps<{ itemId: string }>();
const { theme, toggleTheme } = useTheme();

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
let saveTimer: number | undefined;
let opacitySaveTimer: number | undefined;
let stopListening: (() => void) | undefined;

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
  await loadItem();
  stopListening = await listen<Item>("items-changed", ({ payload }) => {
    if (payload.id !== props.itemId || saveState.value === "saving") return;
    applyItem(payload);
  });
  window.addEventListener("keydown", handleShortcut);
});

onBeforeUnmount(() => {
  if (saveTimer) window.clearTimeout(saveTimer);
  if (opacitySaveTimer) window.clearTimeout(opacitySaveTimer);
  stopListening?.();
  window.removeEventListener("keydown", handleShortcut);
});

watch([title, content], () => {
  if (!ready.value) return;
  scheduleSave();
});

async function loadItem() {
  try {
    applyItem(await itemsApi.get(props.itemId));
    ready.value = true;
  } catch (cause) {
    error.value = errorMessage(cause);
  }
}

function applyItem(next: Item) {
  item.value = next;
  title.value = next.title;
  content.value = next.content;
  color.value = next.color;
  pinned.value = next.window?.alwaysOnTop ?? true;
  backgroundOpacity.value = Math.round((next.window?.backgroundOpacity ?? 1) * 100);
  textOpacity.value = Math.round((next.window?.textOpacity ?? 1) * 100);
}

function scheduleSave() {
  saveState.value = "idle";
  if (saveTimer) window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(saveNow, 550);
}

async function chooseColor(next: ItemColor) {
  if (color.value === next) return;
  color.value = next;
  await saveNow();
}

async function saveNow() {
  if (
    !item.value ||
    (!title.value.trim() &&
      !content.value.trim() &&
      item.value.todoEntries.length === 0)
  ) {
    return;
  }
  if (
    item.value.title === title.value &&
    item.value.content === content.value &&
    item.value.color === color.value
  ) {
    return;
  }

  saveState.value = "saving";
  try {
    const updated = await itemsApi.update({
      id: item.value.id,
      expectedRevision: item.value.revision,
      group: item.value.group,
      title: title.value,
      content: content.value,
      color: color.value,
      tags: item.value.tags,
      dueAt: item.value.dueAt,
      expiresAt: item.value.expiresAt,
    });
    item.value = updated;
    saveState.value = "saved";
    window.setTimeout(() => {
      if (saveState.value === "saved") saveState.value = "idle";
    }, 1200);
  } catch (cause) {
    saveState.value = "error";
    error.value = errorMessage(cause);
    if (error.value.includes("changed elsewhere")) await loadItem();
  }
}

function scheduleOpacitySave() {
  saveState.value = "idle";
  if (opacitySaveTimer) window.clearTimeout(opacitySaveTimer);
  opacitySaveTimer = window.setTimeout(() => void saveOpacityNow(), 180);
}

async function saveOpacityNow() {
  if (!item.value) return;
  if (opacitySaveTimer) {
    window.clearTimeout(opacitySaveTimer);
    opacitySaveTimer = undefined;
  }

  const current = item.value.window;
  if (
    current &&
    Math.round(current.backgroundOpacity * 100) === backgroundOpacity.value &&
    Math.round(current.textOpacity * 100) === textOpacity.value
  ) {
    return;
  }

  saveState.value = "saving";
  try {
    const state = await itemsApi.setStickyOpacity(
      item.value.id,
      backgroundOpacity.value / 100,
      textOpacity.value / 100,
    );
    item.value = { ...item.value, window: state };
    backgroundOpacity.value = Math.round(state.backgroundOpacity * 100);
    textOpacity.value = Math.round(state.textOpacity * 100);
    saveState.value = "saved";
    window.setTimeout(() => {
      if (saveState.value === "saved") saveState.value = "idle";
    }, 1200);
  } catch (cause) {
    saveState.value = "error";
    error.value = errorMessage(cause);
  }
}

async function togglePin() {
  if (!item.value) return;
  try {
    const state = await itemsApi.setStickyPin(item.value.id, !pinned.value);
    pinned.value = state.alwaysOnTop;
  } catch (cause) {
    error.value = errorMessage(cause);
  }
}

async function toggleTodo() {
  if (!item.value || item.value.kind !== "todo") return;
  try {
    await saveNow();
    const status = item.value.status === "completed" ? "active" : "completed";
    applyItem(await itemsApi.setStatus(item.value, status));
  } catch (cause) {
    error.value = errorMessage(cause);
  }
}

function todoMutationIsBusy(): boolean {
  return saveState.value === "saving";
}

async function runTodoMutation(action: (current: Item) => Promise<Item>) {
  if (!item.value || item.value.kind !== "todo" || todoMutationIsBusy()) return;
  await saveNow();
  if (!item.value || todoMutationIsBusy()) return;

  saveState.value = "saving";
  error.value = "";
  try {
    applyItem(await action(item.value));
    saveState.value = "saved";
    window.setTimeout(() => {
      if (saveState.value === "saved") saveState.value = "idle";
    }, 1200);
  } catch (cause) {
    saveState.value = "error";
    error.value = errorMessage(cause);
    await loadItem();
  }
}

async function toggleTodoEntry(entry: TodoEntry) {
  await runTodoMutation((current) =>
    itemsApi.setTodoEntryCompleted(current, entry, !entry.isCompleted),
  );
}

async function renameTodoEntry(entry: TodoEntry, event: Event) {
  const input = event.target as HTMLInputElement;
  const normalized = input.value.trim();
  if (!normalized) {
    input.value = entry.text;
    return;
  }
  if (normalized === entry.text) return;
  await runTodoMutation((current) =>
    itemsApi.updateTodoEntry(current, entry, normalized),
  );
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
  if (!item.value) return;
  if (saveTimer) window.clearTimeout(saveTimer);
  if (opacitySaveTimer) window.clearTimeout(opacitySaveTimer);
  await saveNow();
  await saveOpacityNow();
  await itemsApi.hideSticky(item.value.id);
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
    error.value = errorMessage(cause);
  }
}
</script>

<template>
  <main :class="shellClasses" :style="shellStyle">
    <header class="sticky-titlebar" data-tauri-drag-region>
      <div class="sticky-grip" data-tauri-drag-region>
        <span></span><span></span><span></span>
      </div>
      <div class="sticky-window-actions">
        <button
          :class="{ active: pinned }"
          :title="pinned ? '取消置顶' : '置顶'"
          aria-label="切换置顶"
          @click="togglePin"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="m9 3 6 0-.8 5 3.3 3.2v1.3H13v6l-1 2-1-2v-6H6.5v-1.3L9.8 8 9 3Z" />
          </svg>
        </button>
        <button
          :class="{ active: appearancePanelOpen }"
          title="外观与透明度"
          aria-label="外观与透明度"
          @click="appearancePanelOpen = !appearancePanelOpen"
        >
          <span class="opacity-toggle-icon" aria-hidden="true">&#9680;</span>
        </button>
        <button title="最小化" aria-label="最小化" @click="minimizeWindow">&#8722;</button>
        <button title="隐藏便签" aria-label="隐藏便签" @click="hideWindow">×</button>
      </div>
    </header>

    <section
      v-if="appearancePanelOpen"
      class="opacity-panel"
      aria-label="悬浮窗外观"
      @pointerdown.stop
      @click.stop
    >
      <header>
        <strong>外观</strong>
        <button aria-label="关闭外观设置" @click="appearancePanelOpen = false">×</button>
      </header>
      <div class="theme-control">
        <span>主题</span>
        <button
          :class="{ active: theme === 'black_gold' }"
          :aria-pressed="theme === 'black_gold'"
          @click="switchTheme"
        >
          <span class="theme-chip" aria-hidden="true"></span>
          {{ theme === "black_gold" ? "黑金" : "浅色" }}
        </button>
      </div>
      <div class="sticky-color-control">
        <span>背景色</span>
        <div class="sticky-color-picker">
          <button
            v-for="option in colorOptions"
            :key="option.value"
            :class="[`swatch-${option.value}`, { active: color === option.value }]"
            :title="option.label"
            :aria-label="`背景色：${option.label}`"
            :aria-pressed="color === option.value"
            @click="chooseColor(option.value)"
          >
            <span></span>
          </button>
        </div>
      </div>
      <div class="appearance-divider"></div>
      <label>
        <span>背景</span>
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
        <span>文字</span>
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
      <small>配色与透明度会自动保存；主题同步到全部窗口</small>
    </section>

    <section v-if="item" class="sticky-content">
      <div class="sticky-meta">
        <button
          v-if="item.kind === 'todo'"
          class="sticky-check"
          :class="{ checked: item.status === 'completed' }"
          @click="toggleTodo"
        >&#10003;</button>
        <span class="sticky-kind">
          {{ item.kind === "todo" ? "TODO" : item.kind === "secret" ? "加密临时秘密" : "便签" }}
        </span>
        <span class="sticky-group"># {{ item.group }}</span>
        <span v-if="item.kind === 'todo'" class="sticky-todo-progress">
          {{ todoProgress.completed }}/{{ todoProgress.total }}
        </span>
        <span v-if="item.kind === 'secret'" class="mcp-denied">MCP DENY</span>
      </div>

      <input
        v-model="title"
        class="sticky-title-input"
        maxlength="160"
        aria-label="便签标题"
        placeholder="未命名"
      />

      <section v-if="item.kind === 'todo'" class="sticky-todo-list" aria-label="TODO 清单">
        <div v-if="item.todoEntries.length" class="sticky-todo-items">
          <div v-for="entry in item.todoEntries" :key="entry.id" class="sticky-todo-row">
            <button
              type="button"
              class="sticky-entry-check"
              :class="{ checked: entry.isCompleted }"
              :title="entry.isCompleted ? '恢复此项' : '完成此项'"
              @click="toggleTodoEntry(entry)"
            >
              &#10003;
            </button>
            <input
              :value="entry.text"
              class="sticky-entry-text"
              :class="{ completed: entry.isCompleted }"
              maxlength="500"
              spellcheck="false"
              aria-label="清单项"
              @change="renameTodoEntry(entry, $event)"
            />
            <button
              type="button"
              class="sticky-entry-delete"
              title="删除此项"
              aria-label="删除此项"
              @click="deleteTodoEntry(entry)"
            >
              ×
            </button>
          </div>
        </div>
        <p v-else class="sticky-todo-empty">还没有清单项</p>
        <form class="sticky-todo-add" @submit.prevent="addTodoEntry">
          <input v-model="newTodoText" maxlength="500" placeholder="添加下一项…" aria-label="添加清单项" />
          <button type="submit" :disabled="!newTodoText.trim()" title="添加清单项">＋</button>
        </form>
      </section>

      <template v-if="item.kind === 'secret' && !revealing">
        <button class="secret-cover" @click="revealing = true">
          <span>◇</span>
          <strong>内容已隐藏</strong>
          <small>点击后在此窗口显示</small>
        </button>
      </template>
      <textarea
        v-else
        v-model="content"
        class="sticky-body-input"
        aria-label="便签内容"
        placeholder="写点什么…"
        spellcheck="false"
      ></textarea>

      <button
        v-if="item.kind === 'secret' && revealing"
        class="hide-secret"
        @click="revealing = false"
      >
        隐藏内容
      </button>
    </section>

    <section v-else-if="error" class="sticky-error">{{ error }}</section>
    <section v-else class="sticky-loading">正在打开…</section>

    <footer v-if="item" class="sticky-footer">
      <span class="save-state" :class="saveState">
        {{
          saveState === "saving"
            ? "保存中"
            : saveState === "saved"
              ? "已保存"
              : saveState === "error"
                ? "保存失败"
                : pinned
                  ? "已置顶"
                  : "普通窗口"
        }}
      </span>
    </footer>
  </main>
</template>
