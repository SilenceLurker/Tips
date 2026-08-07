<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { errorMessage, itemsApi } from "../api";
import { useTheme } from "../theme";
import {
  colorOptions,
  kindLabels,
  type Item,
  type ItemColor,
  type ItemKind,
  type McpAccess,
  type TodoEntry,
  type TodoEntryDraftInput,
} from "../types";

type Filter = "all" | ItemKind | "completed" | `group:${string}`;

interface DraftTodoEntry extends TodoEntryDraftInput {
  clientId: string;
}

interface Draft {
  id: string | null;
  revision: number | null;
  kind: ItemKind;
  group: string;
  title: string;
  content: string;
  color: ItemColor;
  tags: string;
  dueAt: string;
  expiresAt: string;
  todoEntries: DraftTodoEntry[];
}

const { theme, toggleTheme } = useTheme();
const items = ref<Item[]>([]);
const loading = ref(true);
const saving = ref(false);
const filter = ref<Filter>("all");
const search = ref("");
const editorOpen = ref(false);
const floatAfterCreate = ref(true);
const toast = ref("");
const error = ref("");
let stopListening: (() => void) | undefined;
let toastTimer: number | undefined;

const newDraftTodoEntry = (
  values: Partial<TodoEntryDraftInput> = {},
): DraftTodoEntry => ({
  clientId: `draft-${Date.now()}-${Math.random().toString(16).slice(2)}`,
  id: values.id ?? null,
  expectedRevision: values.expectedRevision ?? null,
  text: values.text ?? "",
  isCompleted: values.isCompleted ?? false,
});

const emptyDraft = (): Draft => ({
  id: null,
  revision: null,
  kind: "note",
  group: "inbox",
  title: "",
  content: "",
  color: "sun",
  tags: "",
  dueAt: "",
  expiresAt: "",
  todoEntries: [],
});

const draft = ref<Draft>(emptyDraft());

const counts = computed(() => ({
  all: items.value.length,
  note: items.value.filter((item) => item.kind === "note").length,
  todo: items.value.filter((item) => item.kind === "todo" && item.status === "active").length,
  secret: items.value.filter((item) => item.kind === "secret").length,
  completed: items.value.filter((item) => item.status === "completed").length,
}));

const groups = computed(() => {
  const visibleItems = items.value.filter((item) => item.status !== "archived");
  const countsByGroup = new Map<string, number>();
  for (const item of visibleItems) {
    countsByGroup.set(item.group, (countsByGroup.get(item.group) ?? 0) + 1);
  }
  if (!countsByGroup.has("inbox")) countsByGroup.set("inbox", 0);
  return Array.from(countsByGroup.entries())
    .sort(([left], [right]) => left.localeCompare(right, "zh-CN"))
    .map(([key, count]) => ({ key, count }));
});

const currentHeading = computed(() => {
  const current = filter.value;
  if (current === "all") return "我的桌面";
  if (current === "completed") return "已完成";
  if (current.startsWith("group:")) {
    const group = current.slice(6);
    return group === "inbox" ? "收件箱" : group;
  }
  return kindLabels[current as ItemKind];
});

const filteredItems = computed(() => {
  const needle = search.value.trim().toLocaleLowerCase();
  return items.value.filter((item) => {
    const matchesFilter =
      filter.value === "all"
        ? item.status !== "archived"
        : filter.value === "completed"
          ? item.status === "completed"
          : filter.value.startsWith("group:")
            ? item.group === filter.value.slice(6) && item.status !== "archived"
            : item.kind === filter.value && item.status !== "archived";
    if (!matchesFilter) return false;
    if (!needle) return true;
    return [item.title, item.content, item.group, ...item.tags, ...item.todoEntries.map((entry) => entry.text)]
      .join(" ")
      .toLocaleLowerCase()
      .includes(needle);
  });
});

const activeTodos = computed(() =>
  items.value
    .filter((item) => item.kind === "todo" && item.status === "active")
    .sort((a, b) => (a.dueAt ?? Number.MAX_SAFE_INTEGER) - (b.dueAt ?? Number.MAX_SAFE_INTEGER))
    .slice(0, 3),
);

onMounted(async () => {
  await refresh();
  stopListening = await listen<Item>("items-changed", ({ payload }) => upsertItem(payload));
});

onBeforeUnmount(() => {
  stopListening?.();
  if (toastTimer) window.clearTimeout(toastTimer);
});

async function refresh() {
  loading.value = true;
  error.value = "";
  try {
    items.value = await itemsApi.list(false);
  } catch (cause) {
    error.value = errorMessage(cause);
  } finally {
    loading.value = false;
  }
}

function openCreate(kind: ItemKind = "note") {
  draft.value = {
    ...emptyDraft(),
    kind,
    group: filter.value.startsWith("group:") ? filter.value.slice(6) : "inbox",
    color: kind === "secret" ? "lilac" : "sun",
    todoEntries: kind === "todo" ? [newDraftTodoEntry()] : [],
  };
  floatAfterCreate.value = true;
  editorOpen.value = true;
}

function openEdit(item: Item) {
  draft.value = {
    id: item.id,
    revision: item.revision,
    kind: item.kind,
    group: item.group,
    title: item.title,
    content: item.content,
    color: item.color,
    tags: item.tags.join(", "),
    dueAt: toLocalInput(item.dueAt),
    expiresAt: toLocalInput(item.expiresAt),
    todoEntries: item.todoEntries.map((entry) =>
      newDraftTodoEntry({
        id: entry.id,
        expectedRevision: entry.revision,
        text: entry.text,
        isCompleted: entry.isCompleted,
      }),
    ),
  };
  editorOpen.value = true;
}

function selectGroup(group: string) {
  filter.value = `group:${group}`;
}

function groupLabel(group: string) {
  return group === "inbox" ? "收件箱" : group;
}

function selectDraftKind(kind: ItemKind) {
  if (draft.value.id) return;
  draft.value.kind = kind;
  if (kind === "todo" && draft.value.todoEntries.length === 0) {
    draft.value.todoEntries.push(newDraftTodoEntry());
  }
}

function addDraftTodoEntry() {
  if (draft.value.todoEntries.length >= 200) return;
  draft.value.todoEntries.push(newDraftTodoEntry());
}

function removeDraftTodoEntry(index: number) {
  draft.value.todoEntries.splice(index, 1);
}

async function saveDraft() {
  const todoEntries = draft.value.todoEntries
    .filter((entry) => entry.text.trim())
    .map((entry) => ({ ...entry, text: entry.text.trim() }));
  if (!draft.value.title.trim() && !draft.value.content.trim() && todoEntries.length === 0) {
    error.value = "标题、内容和清单项不能同时为空";
    return;
  }

  saving.value = true;
  error.value = "";
  try {
    const tags = draft.value.tags
      .split(/[,，]/)
      .map((tag) => tag.trim())
      .filter(Boolean);

    if (draft.value.id && draft.value.revision !== null) {
      const updated = await itemsApi.update({
        id: draft.value.id,
        expectedRevision: draft.value.revision,
        group: draft.value.group,
        title: draft.value.title,
        content: draft.value.content,
        color: draft.value.color,
        tags,
        dueAt: fromLocalInput(draft.value.dueAt),
        expiresAt: fromLocalInput(draft.value.expiresAt),
        todoEntries:
          draft.value.kind === "todo"
            ? todoEntries.map(({ id, expectedRevision, text, isCompleted }) => ({
                id,
                expectedRevision,
                text,
                isCompleted,
              }))
            : undefined,
      });
      upsertItem(updated);
      notify("已保存");
    } else {
      const mcpAccess: McpAccess = draft.value.kind === "secret" ? "deny" : "inherit";
      const created = await itemsApi.create({
        kind: draft.value.kind,
        group: draft.value.group,
        title: draft.value.title,
        content: draft.value.content,
        color: draft.value.color,
        tags,
        dueAt: fromLocalInput(draft.value.dueAt),
        expiresAt: fromLocalInput(draft.value.expiresAt),
        mcpAccess,
        todoEntries: draft.value.kind === "todo" ? todoEntries.map((entry) => entry.text) : [],
      });
      upsertItem(created);
      if (floatAfterCreate.value) {
        await itemsApi.openSticky(created.id);
        await refresh();
      }
      notify("便签已创建");
    }
    editorOpen.value = false;
  } catch (cause) {
    error.value = errorMessage(cause);
    if (error.value.includes("changed elsewhere")) await refresh();
  } finally {
    saving.value = false;
  }
}

async function toggleTodo(item: Item) {
  try {
    const nextStatus = item.status === "completed" ? "active" : "completed";
    upsertItem(await itemsApi.setStatus(item, nextStatus));
  } catch (cause) {
    error.value = errorMessage(cause);
    await refresh();
  }
}

async function toggleTodoEntry(item: Item, entry: TodoEntry) {
  try {
    upsertItem(await itemsApi.setTodoEntryCompleted(item, entry, !entry.isCompleted));
  } catch (cause) {
    error.value = errorMessage(cause);
    await refresh();
  }
}

function completedTodoCount(item: Item): number {
  return item.todoEntries.filter((entry) => entry.isCompleted).length;
}

async function archive(item: Item) {
  try {
    await itemsApi.setStatus(item, "archived");
    items.value = items.value.filter((entry) => entry.id !== item.id);
    notify("已归档，可在后续归档视图中恢复");
  } catch (cause) {
    error.value = errorMessage(cause);
  }
}

async function floatItem(item: Item) {
  try {
    await itemsApi.openSticky(item.id);
    const local = items.value.find((entry) => entry.id === item.id);
    if (local?.window) local.window.isVisible = true;
    notify("便签已悬浮并保持置顶");
  } catch (cause) {
    error.value = errorMessage(cause);
  }
}

function upsertItem(item: Item) {
  const index = items.value.findIndex((entry) => entry.id === item.id);
  if (index === -1) {
    items.value = [item, ...items.value];
  } else {
    items.value.splice(index, 1, item);
  }
}

function notify(message: string) {
  toast.value = message;
  if (toastTimer) window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    toast.value = "";
  }, 2200);
}

async function switchTheme() {
  try {
    const next = await toggleTheme();
    notify(next === "black_gold" ? "已切换到黑金模式" : "已切换到浅色模式");
  } catch (cause) {
    error.value = errorMessage(cause);
  }
}

function toLocalInput(timestamp: number | null): string {
  if (!timestamp) return "";
  const date = new Date(timestamp);
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

function fromLocalInput(value: string): number | null {
  return value ? new Date(value).getTime() : null;
}

function formatTime(timestamp: number | null): string {
  if (!timestamp) return "";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(timestamp);
}

function isExpired(item: Item): boolean {
  return item.expiresAt !== null && item.expiresAt < Date.now();
}
</script>

<template>
  <div class="main-shell">
    <aside class="sidebar">
      <div class="brand">
        <span class="brand-mark">T</span>
        <div>
          <strong>Tips</strong>
          <small>留在桌面，随手就记</small>
        </div>
      </div>

      <button class="new-note-button" @click="openCreate()">
        <span>＋</span>
        新建便签
      </button>

      <nav class="filter-list" aria-label="便签分类">
        <button :class="{ active: filter === 'all' }" @click="filter = 'all'">
          <span class="nav-icon">&#9638;</span>
          全部
          <em>{{ counts.all }}</em>
        </button>
        <button :class="{ active: filter === 'note' }" @click="filter = 'note'">
          <span class="nav-icon">&#9998;</span>
          便签
          <em>{{ counts.note }}</em>
        </button>
        <button :class="{ active: filter === 'todo' }" @click="filter = 'todo'">
          <span class="nav-icon">&#10003;</span>
          TODO
          <em>{{ counts.todo }}</em>
        </button>
        <button :class="{ active: filter === 'secret' }" @click="filter = 'secret'">
          <span class="nav-icon">◇</span>
          临时秘密
          <em>{{ counts.secret }}</em>
        </button>
        <button :class="{ active: filter === 'completed' }" @click="filter = 'completed'">
          <span class="nav-icon">◎</span>
          已完成
          <em>{{ counts.completed }}</em>
        </button>
      </nav>

      <section class="group-panel">
        <header>
          <span>任务分组</span>
          <small>{{ groups.length }}</small>
        </header>
        <div class="group-list">
          <button v-for="entry in groups" :key="entry.key" :class="{ active: filter === 'group:' + entry.key }" @click="selectGroup(entry.key)">
            <span class="group-icon">#</span>
            <strong :title="entry.key">{{ groupLabel(entry.key) }}</strong>
            <em>{{ entry.count }}</em>
          </button>
        </div>
      </section>

      <section v-if="activeTodos.length" class="today-panel">
        <p>接下来</p>
        <button v-for="todo in activeTodos" :key="todo.id" @click="openEdit(todo)">
          <span></span>
          <div>
            <strong>{{
              todo.title || todo.todoEntries.find((entry) => !entry.isCompleted)?.text || "未命名任务"
            }}</strong>
            <small>{{ groupLabel(todo.group) }} · {{ todo.dueAt ? formatTime(todo.dueAt) : "未设置时间" }}</small>
          </div>
        </button>
      </section>

      <button
        class="theme-toggle"
        :class="{ active: theme === 'black_gold' }"
        :aria-pressed="theme === 'black_gold'"
        @click="switchTheme"
      >
        <span class="theme-toggle-icon">{{ theme === "black_gold" ? "◆" : "☾" }}</span>
        <span>
          <strong>黑金模式</strong>
          <small>{{ theme === "black_gold" ? "暖灰正文 · 暗金点缀" : "切换深色外观" }}</small>
        </span>
        <em>{{ theme === "black_gold" ? "ON" : "OFF" }}</em>
      </button>

      <div class="local-status">
        <span></span>
        <div>
          <strong>仅本机</strong>
          <small>MCP 网络服务未开启</small>
        </div>
      </div>
    </aside>

    <main class="workspace">
      <header class="workspace-header">
        <div>
          <p class="eyebrow">LOCAL-FIRST DESKTOP NOTES</p>
          <h1>{{ currentHeading }}</h1>
        </div>
        <label class="search-box">
          <span>&#128269;</span>
          <input v-model="search" type="search" placeholder="搜索标题、内容、分组、清单项或标签" />
          <kbd>Ctrl K</kbd>
        </label>
      </header>

      <div v-if="error" class="error-banner">
        <span>{{ error }}</span>
        <button @click="error = ''">×</button>
      </div>

      <section class="quick-actions">
        <button @click="openCreate('note')">
          <span class="quick-icon sun">&#9998;</span>
          <span><strong>写一张便签</strong><small>灵感、片段、临时记录</small></span>
        </button>
        <button @click="openCreate('todo')">
          <span class="quick-icon mint">&#10003;</span>
          <span><strong>添加 TODO</strong><small>带截止时间的待办</small></span>
        </button>
        <button @click="openCreate('secret')">
          <span class="quick-icon lilac">◇</span>
          <span><strong>保存临时秘密</strong><small>加密保存，默认 24 小时</small></span>
        </button>
      </section>

      <section class="section-heading">
        <div>
          <h2>便签墙</h2>
          <span>{{ filteredItems.length }} 项</span>
        </div>
        <p>点击悬浮按钮，把它留在其他窗口上方</p>
      </section>

      <div v-if="loading" class="loading-state">
        <span></span>
        正在打开本地便签库…
      </div>

      <section v-else-if="filteredItems.length" class="note-grid">
        <article
          v-for="item in filteredItems"
          :key="item.id"
          class="note-card"
          :class="[`color-${item.color}`, { completed: item.status === 'completed' }]"
        >
          <div class="note-card-top">
            <div class="note-card-classifiers">
              <span class="kind-pill">
                {{ item.kind === "secret" ? "加密" : kindLabels[item.kind] }}
              </span>
              <button class="group-pill" :title="`筛选分组：${item.group}`" @click="selectGroup(item.group)">
                {{ groupLabel(item.group) }}
              </button>
            </div>
            <div class="card-actions">
              <button title="悬浮并置顶" @click="floatItem(item)">&#8599;</button>
              <button title="编辑" @click="openEdit(item)">···</button>
            </div>
          </div>

          <button
            v-if="item.kind === 'todo'"
            class="todo-title"
            :aria-label="item.status === 'completed' ? '恢复任务' : '完成任务'"
            @click="toggleTodo(item)"
          >
            <span :class="{ checked: item.status === 'completed' }">&#10003;</span>
            <strong>{{ item.title || "未命名任务" }}</strong>
          </button>
          <h3 v-else>{{ item.title || (item.kind === "secret" ? "临时秘密" : "未命名便签") }}</h3>

          <div v-if="item.kind === 'todo' && item.todoEntries.length" class="card-todo-list">
            <button
              v-for="entry in item.todoEntries.slice(0, 4)"
              :key="entry.id"
              type="button"
              :title="entry.isCompleted ? '恢复此项' : '完成此项'"
              @click="toggleTodoEntry(item, entry)"
            >
              <span class="card-todo-check" :class="{ checked: entry.isCompleted }">&#10003;</span>
              <span :class="{ completed: entry.isCompleted }">{{ entry.text }}</span>
            </button>
            <div class="card-todo-progress">
              <span>{{ completedTodoCount(item) }}/{{ item.todoEntries.length }} 已完成</span>
              <span v-if="item.todoEntries.length > 4">另有 {{ item.todoEntries.length - 4 }} 项</span>
            </div>
          </div>

          <p v-if="item.kind === 'secret'" class="secret-preview">&bull;&bull;&bull;&bull;&bull;&bull;&bull;&bull;</p>
          <p v-else-if="item.content" class="content-preview">{{ item.content }}</p>
          <p v-else-if="item.kind !== 'todo'" class="content-preview">空便签</p>

          <div v-if="item.tags.length" class="tag-list">
            <span v-for="tag in item.tags.slice(0, 3)" :key="tag">#{{ tag }}</span>
          </div>

          <footer>
            <span v-if="item.kind === 'secret'" :class="{ expired: isExpired(item) }">
              {{ isExpired(item) ? "已过期" : item.expiresAt ? `到期 ${formatTime(item.expiresAt)}` : "MCP 禁止" }}
            </span>
            <span v-else-if="item.dueAt">截止 {{ formatTime(item.dueAt) }}</span>
            <span v-else>{{ formatTime(item.updatedAt) }}</span>
            <button title="归档" @click="archive(item)">归档</button>
          </footer>
        </article>
      </section>

      <section v-else class="empty-state">
        <div class="empty-illustration">
          <span></span><span></span><span></span>
        </div>
        <h2>这里还很安静</h2>
        <p>写下第一张便签，或把一个 TODO 固定在桌面上。</p>
        <button @click="openCreate()">创建第一张便签</button>
      </section>
    </main>

    <div v-if="editorOpen" class="modal-backdrop" @mousedown.self="editorOpen = false">
      <form class="editor-modal" @submit.prevent="saveDraft">
        <header>
          <div>
            <p>{{ draft.id ? "EDIT ITEM" : "NEW ITEM" }}</p>
            <h2>{{ draft.id ? "编辑内容" : "记下此刻" }}</h2>
          </div>
          <button type="button" aria-label="关闭" @click="editorOpen = false">×</button>
        </header>

        <div class="kind-switch" :class="{ disabled: draft.id }">
          <button
            v-for="kind in (['note', 'todo', 'secret'] as ItemKind[])"
            :key="kind"
            type="button"
            :class="{ active: draft.kind === kind }"
            :disabled="Boolean(draft.id)"
            @click="selectDraftKind(kind)"
          >
            {{ kindLabels[kind] }}
          </button>
        </div>

        <label>
          <span>标题</span>
          <input v-model="draft.title" maxlength="160" placeholder="一句话说明这是什么" autofocus />
        </label>

        <section v-if="draft.kind === 'todo'" class="todo-editor">
          <header>
            <div>
              <strong>清单项</strong>
              <span v-if="draft.todoEntries.length">
                {{ draft.todoEntries.filter((entry) => entry.isCompleted).length }}/{{
                  draft.todoEntries.length
                }}
              </span>
            </div>
            <button type="button" :disabled="draft.todoEntries.length >= 200" @click="addDraftTodoEntry">
              ＋ 添加一项
            </button>
          </header>
          <div v-if="draft.todoEntries.length" class="todo-editor-list">
            <div v-for="(entry, index) in draft.todoEntries" :key="entry.clientId" class="todo-editor-row">
              <label class="todo-editor-check" :title="entry.isCompleted ? '恢复此项' : '完成此项'">
                <input v-model="entry.isCompleted" type="checkbox" />
                <span>&#10003;</span>
              </label>
              <input
                v-model="entry.text"
                maxlength="500"
                :class="{ completed: entry.isCompleted }"
                :placeholder="`清单项 ${index + 1}`"
                @keydown.enter.prevent="addDraftTodoEntry"
              />
              <button type="button" class="todo-editor-remove" title="删除此项" @click="removeDraftTodoEntry(index)">
                ×
              </button>
            </div>
          </div>
          <button v-else type="button" class="todo-editor-empty" @click="addDraftTodoEntry">
            暂无清单项，点击添加第一项
          </button>
        </section>

        <label class="content-field">
          <span>{{ draft.kind === "secret" ? "秘密内容" : draft.kind === "todo" ? "补充说明" : "内容" }}</span>
          <textarea
            v-model="draft.content"
            :rows="draft.kind === 'todo' ? 4 : 7"
            :placeholder="draft.kind === 'todo' ? '补充步骤或上下文…' : '现在想到什么？'"
          ></textarea>
        </label>

        <div class="form-row">
          <label>
            <span>分组</span>
            <input v-model="draft.group" maxlength="96" placeholder="inbox 或 project-x/backend" />
          </label>
          <label>
            <span>标签</span>
            <input v-model="draft.tags" placeholder="工作, 灵感" />
          </label>
          <label v-if="draft.kind === 'todo'">
            <span>截止时间</span>
            <input v-model="draft.dueAt" type="datetime-local" />
          </label>
          <label v-if="draft.kind === 'secret'">
            <span>自动过期</span>
            <input v-model="draft.expiresAt" type="datetime-local" />
          </label>
        </div>

        <fieldset class="color-picker">
          <legend>便签颜色</legend>
          <button
            v-for="option in colorOptions"
            :key="option.value"
            type="button"
            :class="[`swatch-${option.value}`, { active: draft.color === option.value }]"
            :title="option.label"
            @click="draft.color = option.value"
          >
            <span></span>
          </button>
        </fieldset>

        <div v-if="draft.kind === 'secret'" class="security-note">
          <span>◇</span>
          <p><strong>内容会加密后写入本地数据库。</strong>MCP 访问在数据层被强制设为 deny。</p>
        </div>

        <footer>
          <label v-if="!draft.id" class="float-option">
            <input v-model="floatAfterCreate" type="checkbox" />
            <span>创建后悬浮并置顶</span>
          </label>
          <div>
            <button type="button" class="secondary" @click="editorOpen = false">取消</button>
            <button type="submit" class="primary" :disabled="saving">
              {{
                saving
                  ? "保存中…"
                  : draft.id
                    ? "保存修改"
                    : draft.kind === "todo"
                      ? "创建清单"
                      : "创建便签"
              }}
            </button>
          </div>
        </footer>
      </form>
    </div>

    <Transition name="toast">
      <div v-if="toast" class="toast">{{ toast }}</div>
    </Transition>
  </div>
</template>
