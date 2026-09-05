<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useI18n } from "vue-i18n";
import { errorMessage, groupsApi, itemsApi, mcpApi, syncApi } from "../api";
import { useLocale } from "../i18n";
import { useTheme } from "../theme";
import {
  type Item,
  type ItemColor,
  type ItemGroup,
  type ItemKind,
  type GroupChangedEvent,
  type LocalePreference,
  type McpAccess,
  type McpClient,
  type McpGroupGrant,
  type McpGroupRole,
  type McpRuntimeStatus,
  type ProvisionedMcpClient,
  type TodoEntry,
  type TodoEntryDraftInput,
} from "../types";

type Filter = "all" | ItemKind | "completed" | "archived" | `group:${string}`;

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

const itemKinds: ItemKind[] = ["note", "todo", "secret"];
const itemColors: ItemColor[] = ["sun", "mint", "sky", "rose", "lilac", "paper"];
const localeOptions: Array<{ value: LocalePreference; labelKey: string }> = [
  { value: "system", labelKey: "locale.system" },
  { value: "zh-CN", labelKey: "locale.zhCN" },
  { value: "en-US", labelKey: "locale.enUS" },
];

const { t, locale } = useI18n({ useScope: "global" });
const { preference: localePreference, setPreference: setLocalePreference } = useLocale();
const { theme, toggleTheme } = useTheme();
const items = ref<Item[]>([]);
const groupRecords = ref<ItemGroup[]>([]);
const loading = ref(true);
const saving = ref(false);
const filter = ref<Filter>("all");
const search = ref("");
const editorOpen = ref(false);
const groupCreatorOpen = ref(false);
const groupNameDraft = ref("");
const groupNameError = ref("");
const groupNameInput = ref<HTMLInputElement | null>(null);
const groupRenamerOpen = ref(false);
const groupRenameSource = ref("");
const groupRenameDraft = ref("");
const groupRenameError = ref("");
const groupRenameInput = ref<HTMLInputElement | null>(null);
const groupSaving = ref(false);
const mcpPanelOpen = ref(false);
const localeSaving = ref(false);
const syncBusy = ref<"export" | "import" | null>(null);
const syncResult = ref("");
const floatAfterCreate = ref(true);
const toast = ref("");
const error = ref("");
const mcpError = ref("");
const mcpLoading = ref(false);
const mcpSaving = ref(false);
const mcpStatus = ref<McpRuntimeStatus | null>(null);
const mcpClients = ref<McpClient[]>([]);
const mcpGrants = ref<McpGroupGrant[]>([]);
const provisionedClient = ref<ProvisionedMcpClient | null>(null);
const mcpDraft = ref<{
  displayName: string;
  groupName: string;
  role: McpGroupRole;
  canManageGroups: boolean;
}>({
  displayName: "",
  groupName: "inbox",
  role: "contributor",
  canManageGroups: false,
});
const mcpGrantDraft = ref<{ clientId: string; groupName: string; role: McpGroupRole }>({
  clientId: "",
  groupName: "inbox",
  role: "contributor",
});
let stopListening: (() => void) | undefined;
let stopMcpListening: (() => void) | undefined;
let stopGroupsListening: (() => void) | undefined;
let stopWindowCloseListening: (() => void) | undefined;
let toastTimer: number | undefined;
let mcpStatusPollTimer: number | undefined;
let mcpStatusPollAttempts = 0;
let mcpProvisionRequestId = 0;
let itemRefreshGeneration = 0;
let groupRefreshGeneration = 0;
let disposed = false;

const MCP_STATUS_POLL_INTERVAL_MS = 500;
const MCP_STATUS_MAX_POLL_ATTEMPTS = 20;

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

const counts = computed(() => {
  const visibleItems = items.value.filter((item) => item.status !== "archived");
  return {
    all: visibleItems.length,
    note: visibleItems.filter((item) => item.kind === "note").length,
    todo: visibleItems.filter((item) => item.kind === "todo" && item.status === "active").length,
    secret: visibleItems.filter((item) => item.kind === "secret").length,
    completed: visibleItems.filter((item) => item.status === "completed").length,
    archived: items.value.filter((item) => item.status === "archived").length,
  };
});

const groups = computed(() => {
  const visibleItems = items.value.filter((item) => item.status !== "archived");
  const countsByGroup = new Map<string, number>();
  for (const group of groupRecords.value) countsByGroup.set(group.key, 0);
  for (const item of visibleItems) {
    countsByGroup.set(item.group, (countsByGroup.get(item.group) ?? 0) + 1);
  }
  if (!countsByGroup.has("inbox")) countsByGroup.set("inbox", 0);
  if (filter.value.startsWith("group:")) {
    const selectedGroup = filter.value.slice(6);
    if (selectedGroup && !countsByGroup.has(selectedGroup)) countsByGroup.set(selectedGroup, 0);
  }
  return Array.from(countsByGroup.entries())
    .sort(([left], [right]) => {
      if (left === "inbox") return -1;
      if (right === "inbox") return 1;
      return left.localeCompare(right, locale.value);
    })
    .map(([key, count]) => ({ key, count }));
});

const selectedGroupKey = computed(() =>
  filter.value.startsWith("group:") ? filter.value.slice(6) : null,
);

const canRenameSelectedGroup = computed(
  () => Boolean(selectedGroupKey.value && selectedGroupKey.value !== "inbox"),
);

const selectedMcpGrant = computed(() =>
  mcpGrants.value.find(
    (grant) =>
      grant.clientId === mcpGrantDraft.value.clientId &&
      grant.group === mcpGrantDraft.value.groupName,
  ),
);

const currentHeading = computed(() => {
  const current = filter.value;
  if (current === "all") return t("main.headings.desktop");
  if (current === "completed") return t("main.nav.completed");
  if (current === "archived") return t("main.nav.archived");
  if (current.startsWith("group:")) {
    const group = current.slice(6);
    return groupLabel(group);
  }
  return kindLabel(current as ItemKind);
});

const filteredItems = computed(() => {
  const needle = localizedLowerCase(search.value.trim());
  return items.value.filter((item) => {
    const matchesFilter =
      filter.value === "all"
        ? item.status !== "archived"
        : filter.value === "completed"
          ? item.status === "completed"
          : filter.value === "archived"
            ? item.status === "archived"
          : filter.value.startsWith("group:")
            ? item.group === filter.value.slice(6) && item.status !== "archived"
            : item.kind === filter.value && item.status !== "archived";
    if (!matchesFilter) return false;
    if (!needle) return true;
    return localizedLowerCase(
      [item.title, item.content, item.group, ...item.tags, ...item.todoEntries.map((entry) => entry.text)].join(" "),
    ).includes(needle);
  });
});

const activeTodos = computed(() =>
  items.value
    .filter((item) => item.kind === "todo" && item.status === "active")
    .sort((a, b) => (a.dueAt ?? Number.MAX_SAFE_INTEGER) - (b.dueAt ?? Number.MAX_SAFE_INTEGER))
    .slice(0, 3),
);

onMounted(async () => {
  const unlisteners = await Promise.all([
    listen<Item>("items-changed", ({ payload }) => handleItemChanged(payload)),
    listen("mcp-items-changed", reloadItemsFromMcp),
    listen<GroupChangedEvent>("groups-changed", ({ payload }) => handleGroupChanged(payload)),
    getCurrentWindow().onCloseRequested(handleMainWindowClose),
  ]);
  if (disposed) {
    for (const unlisten of unlisteners) unlisten();
    return;
  }
  [stopListening, stopMcpListening, stopGroupsListening, stopWindowCloseListening] = unlisteners;
  await Promise.all([refresh(), refreshMcpStatus(true)]);
});

onBeforeUnmount(() => {
  disposed = true;
  mcpProvisionRequestId += 1;
  itemRefreshGeneration += 1;
  groupRefreshGeneration += 1;
  stopListening?.();
  stopMcpListening?.();
  stopGroupsListening?.();
  stopWindowCloseListening?.();
  if (toastTimer) window.clearTimeout(toastTimer);
  clearMcpStatusPolling();
});

async function refresh() {
  await Promise.all([refreshItems(true), refreshGroups()]);
}

async function refreshGroups(errorPrefix = "") {
  const generation = ++groupRefreshGeneration;
  try {
    const nextGroups = await groupsApi.list();
    if (!disposed && generation === groupRefreshGeneration) {
      groupRecords.value = nextGroups;
    }
  } catch (cause) {
    if (!disposed && generation === groupRefreshGeneration) {
      error.value = `${errorPrefix}${errorMessage(cause, t("errors.unknown"))}`;
    }
  }
}

async function refreshItems(showLoading: boolean, errorPrefix = "") {
  const generation = ++itemRefreshGeneration;
  if (showLoading) loading.value = true;
  if (!errorPrefix) error.value = "";
  try {
    const nextItems = await itemsApi.list(true);
    if (!disposed && generation === itemRefreshGeneration) {
      items.value = nextItems;
    }
  } catch (cause) {
    if (!disposed && generation === itemRefreshGeneration) {
      error.value = `${errorPrefix}${errorMessage(cause, t("errors.unknown"))}`;
    }
  } finally {
    if (!disposed && generation === itemRefreshGeneration) {
      loading.value = false;
    }
  }
}

async function reloadItemsFromMcp() {
  await refreshItems(false, t("main.errors.mcpRefreshPrefix"));
}

async function handleGroupChanged(change: GroupChangedEvent) {
  if (change.previousKey && selectedGroupKey.value === change.previousKey) {
    selectGroup(change.group.key);
  }
  const refreshes: Promise<unknown>[] = [
    refreshGroups(t("main.errors.listSyncPrefix")),
    refreshItems(false, t("main.errors.listSyncPrefix")),
  ];
  if (mcpPanelOpen.value) {
    refreshes.push(
      mcpApi.listGroupGrants().then((grants) => {
        if (!disposed && mcpPanelOpen.value) mcpGrants.value = grants;
      }),
    );
  }
  await Promise.all(refreshes);
  ensureMcpGroupSelections();
}

function handleItemChanged(item: Item) {
  upsertItem(item);
  void refreshItems(false, t("main.errors.listSyncPrefix"));
}

function clearMcpStatusPolling() {
  if (mcpStatusPollTimer !== undefined) {
    window.clearTimeout(mcpStatusPollTimer);
    mcpStatusPollTimer = undefined;
  }
}

function scheduleMcpStatusPolling() {
  clearMcpStatusPolling();
  if (disposed) return;
  const status = mcpStatus.value;
  if (
    !status?.enabled ||
    status.running ||
    status.error ||
    mcpStatusPollAttempts >= MCP_STATUS_MAX_POLL_ATTEMPTS
  ) {
    return;
  }

  mcpStatusPollTimer = window.setTimeout(() => {
    mcpStatusPollTimer = undefined;
    mcpStatusPollAttempts += 1;
    void refreshMcpStatus();
  }, MCP_STATUS_POLL_INTERVAL_MS);
}

async function refreshMcpStatus(resetPolling = false) {
  if (resetPolling) {
    clearMcpStatusPolling();
    mcpStatusPollAttempts = 0;
  }
  try {
    const status = await mcpApi.getRuntimeStatus();
    if (!disposed) mcpStatus.value = status;
  } catch (cause) {
    if (!disposed) {
      mcpStatus.value = {
        enabled: false,
        running: false,
        endpoint: "http://127.0.0.1:4784/mcp",
        error: errorMessage(cause, t("errors.unknown")),
      };
    }
  } finally {
    if (!disposed) scheduleMcpStatusPolling();
  }
}

async function openMcpPanel() {
  mcpPanelOpen.value = true;
  provisionedClient.value = null;
  mcpError.value = "";
  mcpLoading.value = true;
  try {
    const [status, clients, grants, availableGroups] = await Promise.all([
      mcpApi.getRuntimeStatus(),
      mcpApi.listClients(),
      mcpApi.listGroupGrants(),
      groupsApi.list(),
    ]);
    mcpStatus.value = status;
    mcpStatusPollAttempts = 0;
    scheduleMcpStatusPolling();
    mcpClients.value = clients;
    mcpGrants.value = grants;
    groupRecords.value = availableGroups;
    ensureMcpGroupSelections();
  } catch (cause) {
    mcpError.value = errorMessage(cause, t("errors.unknown"));
  } finally {
    mcpLoading.value = false;
  }
}

function closeMcpPanel() {
  if (mcpSaving.value) {
    mcpError.value = t("mcp.errors.waitForOperation");
    return;
  }
  discardMcpPanelState();
}

function handleMainWindowClose() {
  discardMcpPanelState();
}

function discardMcpPanelState() {
  mcpProvisionRequestId += 1;
  mcpPanelOpen.value = false;
  provisionedClient.value = null;
  mcpError.value = "";
}

function ensureMcpGroupSelections(preferredClientId?: string) {
  const availableClientIds = new Set(mcpClients.value.map((client) => client.id));
  if (preferredClientId && availableClientIds.has(preferredClientId)) {
    mcpGrantDraft.value.clientId = preferredClientId;
  } else if (!availableClientIds.has(mcpGrantDraft.value.clientId)) {
    mcpGrantDraft.value.clientId = mcpClients.value[0]?.id ?? "";
  }

  const availableGroupNames = new Set(groupRecords.value.map((group) => group.key));
  const fallbackGroup = availableGroupNames.has("inbox")
    ? "inbox"
    : (groupRecords.value[0]?.key ?? "");
  if (!availableGroupNames.has(mcpDraft.value.groupName)) {
    mcpDraft.value.groupName = fallbackGroup;
  }
  if (!availableGroupNames.has(mcpGrantDraft.value.groupName)) {
    mcpGrantDraft.value.groupName = fallbackGroup;
  }
  syncMcpGrantRole();
}

function syncMcpGrantRole() {
  const existing = selectedMcpGrant.value;
  if (existing) mcpGrantDraft.value.role = existing.role;
}

function editMcpGrant(grant: McpGroupGrant) {
  mcpGrantDraft.value = {
    clientId: grant.clientId,
    groupName: grant.group,
    role: grant.role,
  };
}

async function saveMcpGroupGrant() {
  const { clientId, groupName, role } = mcpGrantDraft.value;
  if (!clientId || !groupName) {
    mcpError.value = t("mcp.errors.grantSelectionRequired");
    return;
  }

  const updating = Boolean(selectedMcpGrant.value);
  mcpSaving.value = true;
  mcpError.value = "";
  try {
    const grant = await mcpApi.setGroupGrant(clientId, groupName, role);
    mcpGrants.value = await mcpApi.listGroupGrants();
    mcpGrantDraft.value.role = grant.role;
    notify(t(updating ? "mcp.feedback.grantUpdated" : "mcp.feedback.grantAdded"));
  } catch (cause) {
    mcpError.value = errorMessage(cause, t("errors.unknown"));
  } finally {
    mcpSaving.value = false;
  }
}

async function provisionMcpClient() {
  const displayName = mcpDraft.value.displayName.trim();
  const groupName = mcpDraft.value.groupName.trim();
  if (!displayName || !groupName) {
    mcpError.value = t("mcp.errors.requiredFields");
    return;
  }

  mcpSaving.value = true;
  const requestId = ++mcpProvisionRequestId;
  mcpError.value = "";
  provisionedClient.value = null;
  try {
    const result = await mcpApi.provisionClient({
      displayName,
      groupName,
      role: mcpDraft.value.role,
      canManageGroups: mcpDraft.value.canManageGroups,
    });
    if (requestId !== mcpProvisionRequestId || !mcpPanelOpen.value) {
      try {
        await mcpApi.revokeClient(result.client.id);
      } catch {
        // The main view is already leaving; never put a late bearer token back into UI state.
      }
      return;
    }
    provisionedClient.value = result;
    mcpClients.value = await mcpApi.listClients();
    mcpGrants.value = await mcpApi.listGroupGrants();
    await refreshGroups();
    mcpGrantDraft.value = {
      clientId: result.client.id,
      groupName: result.grant.group,
      role: result.grant.role,
    };
    ensureMcpGroupSelections(result.client.id);
    mcpDraft.value.displayName = "";
    mcpDraft.value.canManageGroups = false;
    notify(t("mcp.feedback.clientCreated"));
  } catch (cause) {
    mcpError.value = errorMessage(cause, t("errors.unknown"));
  } finally {
    mcpSaving.value = false;
  }
}

async function changeMcpGroupManagement(client: McpClient, event: Event) {
  const input = event.currentTarget as HTMLInputElement;
  const allowed = input.checked;
  mcpSaving.value = true;
  mcpError.value = "";
  try {
    await mcpApi.setGroupManagement(client.id, allowed);
    mcpClients.value = await mcpApi.listClients();
    notify(
      t(
        allowed
          ? "mcp.feedback.groupManagementEnabled"
          : "mcp.feedback.groupManagementDisabled",
      ),
    );
  } catch (cause) {
    input.checked = client.canManageGroups;
    mcpError.value = errorMessage(cause, t("errors.unknown"));
  } finally {
    mcpSaving.value = false;
  }
}

async function revokeMcpClient(client: McpClient) {
  if (!window.confirm(t("mcp.confirm.revokeClient", { name: client.displayName }))) return;
  mcpSaving.value = true;
  mcpError.value = "";
  try {
    await mcpApi.revokeClient(client.id);
    mcpClients.value = await mcpApi.listClients();
    mcpGrants.value = await mcpApi.listGroupGrants();
    ensureMcpGroupSelections();
    notify(t("mcp.feedback.clientRevoked"));
  } catch (cause) {
    mcpError.value = errorMessage(cause, t("errors.unknown"));
  } finally {
    mcpSaving.value = false;
  }
}

async function revokeMcpGrant(grant: McpGroupGrant) {
  if (!window.confirm(t("mcp.confirm.revokeGrant", { group: grant.group }))) return;
  mcpSaving.value = true;
  mcpError.value = "";
  try {
    await mcpApi.revokeGroupGrant(grant.clientId, grant.group);
    mcpGrants.value = await mcpApi.listGroupGrants();
    syncMcpGrantRole();
    notify(t("mcp.feedback.grantRevoked"));
  } catch (cause) {
    mcpError.value = errorMessage(cause, t("errors.unknown"));
  } finally {
    mcpSaving.value = false;
  }
}

function grantsForClient(clientId: string): McpGroupGrant[] {
  return mcpGrants.value.filter((grant) => grant.clientId === clientId);
}

function roleLabel(role: McpGroupRole): string {
  return t(`mcp.roles.${role}`);
}

function formatMcpTime(timestamp: number | null): string {
  if (!timestamp || !Number.isFinite(timestamp)) return t("date.never");
  try {
    return new Intl.DateTimeFormat(locale.value, {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    }).format(timestamp);
  } catch {
    return t("date.invalid");
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
  if (groupRenamerOpen.value && groupRenameSource.value !== group) closeGroupRenamer();
  filter.value = `group:${group}`;
}

function toggleGroupCreator() {
  groupCreatorOpen.value = !groupCreatorOpen.value;
  groupNameError.value = "";
  if (!groupCreatorOpen.value) {
    groupNameDraft.value = "";
    return;
  }
  closeGroupRenamer();
  void nextTick(() => groupNameInput.value?.focus());
}

async function createGroup() {
  const group = normalizeGroupName(groupNameDraft.value);
  if (!group) {
    groupNameError.value = t("main.groups.invalidName");
    return;
  }
  if (groups.value.some((entry) => entry.key === group)) {
    groupNameError.value = t("main.groups.alreadyExists");
    return;
  }

  groupSaving.value = true;
  groupNameError.value = "";
  try {
    const created = await groupsApi.create(group);
    await refreshGroups();
    groupCreatorOpen.value = false;
    groupNameDraft.value = "";
    selectGroup(created.key);
    notify(t("main.groups.created", { group: groupLabel(created.key) }));
  } catch (cause) {
    groupNameError.value = errorMessage(cause, t("errors.unknown"));
  } finally {
    groupSaving.value = false;
  }
}

function openSelectedGroupRenamer() {
  const group = selectedGroupKey.value;
  if (!group || group === "inbox") return;
  groupCreatorOpen.value = false;
  groupNameDraft.value = "";
  groupNameError.value = "";
  groupRenameSource.value = group;
  groupRenameDraft.value = group;
  groupRenameError.value = "";
  groupRenamerOpen.value = true;
  void nextTick(() => {
    groupRenameInput.value?.focus();
    groupRenameInput.value?.select();
  });
}

function closeGroupRenamer() {
  groupRenamerOpen.value = false;
  groupRenameSource.value = "";
  groupRenameDraft.value = "";
  groupRenameError.value = "";
}

async function renameGroup() {
  const source = groupRenameSource.value;
  const newGroup = normalizeGroupName(groupRenameDraft.value);
  if (!source || !newGroup) {
    groupRenameError.value = t("main.groups.invalidName");
    return;
  }
  if (source === newGroup) {
    groupRenameError.value = t("main.groups.renameUnchanged");
    return;
  }
  if (groups.value.some((entry) => entry.key === newGroup)) {
    groupRenameError.value = t("main.groups.alreadyExists");
    return;
  }

  groupSaving.value = true;
  groupRenameError.value = "";
  try {
    const renamed = await groupsApi.rename(source, newGroup);
    selectGroup(renamed.key);
    closeGroupRenamer();
    await Promise.all([refreshGroups(), refreshItems(false)]);
    notify(t("main.groups.renamed", { group: groupLabel(renamed.key) }));
  } catch (cause) {
    groupRenameError.value = errorMessage(cause, t("errors.unknown"));
  } finally {
    groupSaving.value = false;
  }
}

function groupLabel(group: string) {
  return group === "inbox" ? t("main.groups.inbox") : group;
}

function normalizeGroupName(value: string): string | null {
  const normalized = value.trim().toLowerCase();
  if (!normalized || Array.from(normalized).length > 96) return null;
  if (!/^[\p{L}\p{N}._/-]+$/u.test(normalized)) return null;
  if (normalized.split("/").some((segment) => !segment || segment === "." || segment === "..")) return null;
  return normalized;
}

function kindLabel(kind: ItemKind): string {
  return t(`items.kinds.${kind}`);
}

function colorLabel(color: ItemColor): string {
  return t(`items.colors.${color}`);
}

function transportLabel(transport: McpClient["transport"]): string {
  return t(`mcp.transports.${transport}`);
}

function localizedLowerCase(value: string): string {
  try {
    return value.toLocaleLowerCase(locale.value);
  } catch {
    return value.toLocaleLowerCase();
  }
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
    error.value = t("main.validation.itemRequired");
    return;
  }
  const normalizedGroup = normalizeGroupName(draft.value.group);
  if (!normalizedGroup) {
    error.value = t("main.validation.groupInvalid");
    return;
  }
  draft.value.group = normalizedGroup;

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
      notify(t("main.feedback.saved"));
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
      notify(t("main.feedback.created"));
    }
    editorOpen.value = false;
  } catch (cause) {
    error.value = errorMessage(cause, t("errors.unknown"));
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
    error.value = errorMessage(cause, t("errors.unknown"));
    await refresh();
  }
}

async function toggleTodoEntry(item: Item, entry: TodoEntry) {
  try {
    upsertItem(await itemsApi.setTodoEntryCompleted(item, entry, !entry.isCompleted));
  } catch (cause) {
    error.value = errorMessage(cause, t("errors.unknown"));
    await refresh();
  }
}

function completedTodoCount(item: Item): number {
  return item.todoEntries.filter((entry) => entry.isCompleted).length;
}

async function archive(item: Item) {
  try {
    upsertItem(await itemsApi.setStatus(item, "archived"));
    notify(t("main.feedback.archived"));
  } catch (cause) {
    error.value = errorMessage(cause, t("errors.unknown"));
  }
}

async function restoreArchived(item: Item) {
  try {
    upsertItem(await itemsApi.setStatus(item, "active"));
    notify(t("main.feedback.restored"));
  } catch (cause) {
    error.value = errorMessage(cause, t("errors.unknown"));
    await refresh();
  }
}

async function floatItem(item: Item) {
  try {
    await itemsApi.openSticky(item.id);
    const local = items.value.find((entry) => entry.id === item.id);
    if (local?.window) local.window.isVisible = true;
    notify(t("main.feedback.floated"));
  } catch (cause) {
    error.value = errorMessage(cause, t("errors.unknown"));
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

async function exportProgress() {
  if (syncBusy.value) return;
  syncBusy.value = "export";
  syncResult.value = "";
  error.value = "";
  try {
    const result = await syncApi.exportProgress();
    if (!result) return;
    syncResult.value = t("sync.feedback.exported", {
      groups: result.groupCount,
      items: result.itemCount,
      excluded: result.excludedPrivateCount,
    });
    notify(syncResult.value);
  } catch (cause) {
    error.value = t("sync.errors.exportFailed", {
      error: errorMessage(cause, t("errors.unknown")),
    });
  } finally {
    syncBusy.value = null;
  }
}

async function importProgress() {
  if (syncBusy.value) return;
  syncBusy.value = "import";
  syncResult.value = "";
  error.value = "";
  try {
    const result = await syncApi.importProgress();
    if (!result) return;
    await refresh();
    ensureMcpGroupSelections();
    syncResult.value = t("sync.feedback.imported", {
      groups: result.groupsAdded,
      added: result.itemsAdded,
      updated: result.itemsUpdated,
      unchanged: result.itemsUnchanged,
      newer: result.newerLocalItems,
      conflicts: result.conflicts,
    });
    notify(syncResult.value);
  } catch (cause) {
    error.value = t("sync.errors.importFailed", {
      error: errorMessage(cause, t("errors.unknown")),
    });
  } finally {
    syncBusy.value = null;
  }
}

async function switchTheme() {
  try {
    const next = await toggleTheme();
    notify(t(next === "black_gold" ? "main.feedback.themeBlackGold" : "main.feedback.themeLight"));
  } catch (cause) {
    error.value = errorMessage(cause, t("errors.unknown"));
  }
}

async function changeLocale(event: Event) {
  const select = event.currentTarget as HTMLSelectElement;
  const next = select.value as LocalePreference;
  if (next === localePreference.value || localeSaving.value) return;

  localeSaving.value = true;
  try {
    await setLocalePreference(next);
    notify(t("main.feedback.languageChanged"));
  } catch (cause) {
    select.value = localePreference.value;
    error.value = t("locale.saveFailed", {
      error: errorMessage(cause, t("errors.unknown")),
    });
  } finally {
    localeSaving.value = false;
  }
}

function toLocalInput(timestamp: number | null): string {
  if (!timestamp || !Number.isFinite(timestamp)) return "";
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return "";
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  try {
    return local.toISOString().slice(0, 16);
  } catch {
    return "";
  }
}

function fromLocalInput(value: string): number | null {
  if (!value) return null;
  const timestamp = new Date(value).getTime();
  return Number.isFinite(timestamp) ? timestamp : null;
}

function formatTime(timestamp: number | null): string {
  if (!timestamp || !Number.isFinite(timestamp)) return "";
  try {
    return new Intl.DateTimeFormat(locale.value, {
      month: "numeric",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }).format(timestamp);
  } catch {
    return t("date.invalid");
  }
}

function isExpired(item: Item): boolean {
  return item.expiresAt !== null && item.expiresAt < Date.now();
}
</script>

<template>
  <div class="main-shell">
    <aside class="sidebar">
      <div class="sidebar-scroll">
      <div class="brand">
        <span class="brand-mark">T</span>
        <div>
          <strong>Tips</strong>
          <small>{{ t("main.brand.tagline") }}</small>
        </div>
      </div>

      <button class="new-note-button" @click="openCreate()">
        <span>＋</span>
        {{ t("main.actions.newNote") }}
      </button>

      <nav class="filter-list" :aria-label="t('main.nav.label')">
        <button :class="{ active: filter === 'all' }" @click="filter = 'all'">
          <span class="nav-icon">&#9638;</span>
          {{ t("main.nav.all") }}
          <em>{{ counts.all }}</em>
        </button>
        <button :class="{ active: filter === 'note' }" @click="filter = 'note'">
          <span class="nav-icon">&#9998;</span>
          {{ kindLabel("note") }}
          <em>{{ counts.note }}</em>
        </button>
        <button :class="{ active: filter === 'todo' }" @click="filter = 'todo'">
          <span class="nav-icon">&#10003;</span>
          {{ kindLabel("todo") }}
          <em>{{ counts.todo }}</em>
        </button>
        <button :class="{ active: filter === 'secret' }" @click="filter = 'secret'">
          <span class="nav-icon">◇</span>
          {{ kindLabel("secret") }}
          <em>{{ counts.secret }}</em>
        </button>
        <button :class="{ active: filter === 'completed' }" @click="filter = 'completed'">
          <span class="nav-icon">◎</span>
          {{ t("main.nav.completed") }}
          <em>{{ counts.completed }}</em>
        </button>
        <button :class="{ active: filter === 'archived' }" @click="filter = 'archived'">
          <span class="nav-icon">&#9635;</span>
          {{ t("main.nav.archived") }}
          <em>{{ counts.archived }}</em>
        </button>
      </nav>

      <section class="group-panel">
        <header>
          <span>{{ t("main.groups.title") }}</span>
          <div>
            <small>{{ groups.length }}</small>
            <button
              v-if="canRenameSelectedGroup"
              type="button"
              class="group-create-toggle"
              :class="{ active: groupRenamerOpen }"
              :disabled="groupSaving"
              :aria-label="t('main.groups.rename')"
              :title="t('main.groups.rename')"
              @click="groupRenamerOpen ? closeGroupRenamer() : openSelectedGroupRenamer()"
            >
              &#9998;
            </button>
            <button
              type="button"
              class="group-create-toggle"
              :class="{ active: groupCreatorOpen }"
              :disabled="groupSaving"
              :aria-label="t(groupCreatorOpen ? 'main.groups.cancelCreate' : 'main.groups.add')"
              :title="t(groupCreatorOpen ? 'main.groups.cancelCreate' : 'main.groups.add')"
              @click="toggleGroupCreator"
            >
              {{ groupCreatorOpen ? "×" : "+" }}
            </button>
          </div>
        </header>
        <form v-if="groupCreatorOpen" class="group-create-form" @submit.prevent="createGroup">
          <div>
            <input
              ref="groupNameInput"
              v-model="groupNameDraft"
              maxlength="96"
              autocomplete="off"
              spellcheck="false"
              :aria-label="t('main.groups.nameLabel')"
              :placeholder="t('main.groups.namePlaceholder')"
              @input="groupNameError = ''"
              @keydown.esc.prevent="toggleGroupCreator"
            />
            <button type="submit" :disabled="groupSaving" :aria-label="t('main.groups.create')" :title="t('main.groups.create')">✓</button>
          </div>
          <small v-if="groupNameError" role="alert">{{ groupNameError }}</small>
          <small v-else>{{ t("main.groups.createHint") }}</small>
        </form>
        <form v-if="groupRenamerOpen" class="group-create-form" @submit.prevent="renameGroup">
          <div>
            <input
              ref="groupRenameInput"
              v-model="groupRenameDraft"
              maxlength="96"
              autocomplete="off"
              spellcheck="false"
              :aria-label="t('main.groups.renameLabel')"
              :placeholder="t('main.groups.renamePlaceholder')"
              @input="groupRenameError = ''"
              @keydown.esc.prevent="closeGroupRenamer"
            />
            <button type="submit" :disabled="groupSaving" :aria-label="t('main.groups.rename')" :title="t('main.groups.rename')">✓</button>
          </div>
          <small v-if="groupRenameError" role="alert">{{ groupRenameError }}</small>
          <small v-else>{{ t("main.groups.renameHint", { group: groupLabel(groupRenameSource) }) }}</small>
        </form>
        <div class="group-list">
          <button
            v-for="entry in groups"
            :key="entry.key"
            type="button"
            :class="{ active: filter === 'group:' + entry.key }"
            @click="selectGroup(entry.key)"
          >
            <span class="group-icon">#</span>
            <strong :title="entry.key">{{ groupLabel(entry.key) }}</strong>
            <em>{{ entry.count }}</em>
          </button>
        </div>
      </section>

      <section v-if="activeTodos.length" class="today-panel">
        <p>{{ t("main.upNext.title") }}</p>
        <button v-for="todo in activeTodos" :key="todo.id" @click="openEdit(todo)">
          <span></span>
          <div>
            <strong>{{
              todo.title || todo.todoEntries.find((entry) => !entry.isCompleted)?.text || t("items.empty.untitledTask")
            }}</strong>
            <small>{{ groupLabel(todo.group) }} · {{ todo.dueAt ? formatTime(todo.dueAt) : t("main.upNext.noDueDate") }}</small>
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
          <strong>{{ t("main.theme.title") }}</strong>
          <small>{{ t(theme === "black_gold" ? "main.theme.activeDescription" : "main.theme.inactiveDescription") }}</small>
        </span>
        <em>{{ t(theme === "black_gold" ? "common.on" : "common.off") }}</em>
      </button>

      <label class="locale-selector">
        <span>{{ t("locale.label") }}</span>
        <select :value="localePreference" :disabled="localeSaving" @change="changeLocale">
          <option v-for="option in localeOptions" :key="option.value" :value="option.value">
            {{ t(option.labelKey) }}
          </option>
        </select>
      </label>

      <section class="sync-panel" :aria-busy="Boolean(syncBusy)">
        <div class="sync-panel-heading">
          <span>
            <strong>{{ t("sync.card.title") }}</strong>
            <small>{{ t("sync.card.description") }}</small>
          </span>
        </div>
        <div class="sync-actions">
          <button type="button" :disabled="Boolean(syncBusy)" @click="exportProgress">
            <span>↑</span>
            {{ t(syncBusy === "export" ? "sync.actions.exporting" : "sync.actions.export") }}
          </button>
          <button type="button" :disabled="Boolean(syncBusy)" @click="importProgress">
            <span>↓</span>
            {{ t(syncBusy === "import" ? "sync.actions.importing" : "sync.actions.import") }}
          </button>
        </div>
        <small v-if="syncResult" class="sync-result" aria-live="polite">{{ syncResult }}</small>
        <small class="sync-warning">{{ t("sync.card.warning") }}</small>
      </section>

      <button
        type="button"
        class="local-status mcp-status-card"
        :class="{ running: mcpStatus?.running, failed: Boolean(mcpStatus?.error) }"
        @click="openMcpPanel"
      >
        <span class="mcp-status-dot"></span>
        <div>
          <strong>{{ t(mcpStatus?.running ? "mcp.card.localTitle" : "mcp.card.serviceTitle") }}</strong>
          <small v-if="mcpStatus?.error">{{ t("mcp.card.startError") }}</small>
          <small v-else-if="mcpStatus?.running">{{ mcpStatus.endpoint }}</small>
          <small v-else>{{ t(mcpStatus?.enabled ? "mcp.card.starting" : "mcp.card.disabled") }}</small>
        </div>
        <em>{{ t("mcp.card.manage") }}</em>
      </button>
      </div>
    </aside>

    <main class="workspace">
      <header class="workspace-header">
        <div>
          <p class="eyebrow">{{ t("main.eyebrow") }}</p>
          <h1>{{ currentHeading }}</h1>
        </div>
        <label class="search-box">
          <span>&#128269;</span>
          <input v-model="search" type="search" :placeholder="t('main.search.placeholder')" />
          <kbd>Ctrl K</kbd>
        </label>
      </header>

      <div v-if="error" class="error-banner">
        <span>{{ error }}</span>
        <button :aria-label="t('common.dismiss')" @click="error = ''">×</button>
      </div>

      <section v-if="filter !== 'archived'" class="quick-actions">
        <button @click="openCreate('note')">
          <span class="quick-icon sun">&#9998;</span>
          <span><strong>{{ t("main.quick.noteTitle") }}</strong><small>{{ t("main.quick.noteDescription") }}</small></span>
        </button>
        <button @click="openCreate('todo')">
          <span class="quick-icon mint">&#10003;</span>
          <span><strong>{{ t("main.quick.todoTitle") }}</strong><small>{{ t("main.quick.todoDescription") }}</small></span>
        </button>
        <button @click="openCreate('secret')">
          <span class="quick-icon lilac">◇</span>
          <span><strong>{{ t("main.quick.secretTitle") }}</strong><small>{{ t("main.quick.secretDescription") }}</small></span>
        </button>
      </section>

      <section class="section-heading">
        <div>
          <h2>{{ t(filter === "archived" ? "main.archive.wallTitle" : "main.wall.title") }}</h2>
          <span>{{ t("main.wall.count", { count: filteredItems.length }) }}</span>
        </div>
        <p>{{ t(filter === "archived" ? "main.archive.hint" : "main.wall.hint") }}</p>
      </section>

      <div v-if="loading" class="loading-state">
        <span></span>
        {{ t("main.wall.loading") }}
      </div>

      <section v-else-if="filteredItems.length" class="note-grid">
        <article
          v-for="item in filteredItems"
          :key="item.id"
          class="note-card"
          :class="[`color-${item.color}`, { completed: item.status === 'completed', archived: item.status === 'archived' }]"
        >
          <div class="note-card-top">
            <div class="note-card-classifiers">
              <span class="kind-pill">
                {{ item.kind === "secret" ? t("items.labels.encrypted") : kindLabel(item.kind) }}
              </span>
              <button class="group-pill" :title="t('items.actions.filterGroup', { group: item.group })" @click="selectGroup(item.group)">
                {{ groupLabel(item.group) }}
              </button>
            </div>
            <div v-if="item.status !== 'archived'" class="card-actions">
              <button :title="t('items.actions.float')" @click="floatItem(item)">&#8599;</button>
              <button :title="t('items.actions.edit')" @click="openEdit(item)">···</button>
            </div>
          </div>

          <button
            v-if="item.kind === 'todo' && item.status !== 'archived'"
            class="todo-title"
            :aria-label="t(item.status === 'completed' ? 'items.actions.restoreTask' : 'items.actions.completeTask')"
            @click="toggleTodo(item)"
          >
            <span :class="{ checked: item.status === 'completed' }">&#10003;</span>
            <strong>{{ item.title || t("items.empty.untitledTask") }}</strong>
          </button>
          <h3 v-else>
            {{
              item.title ||
              t(
                item.kind === "todo"
                  ? "items.empty.untitledTask"
                  : item.kind === "secret"
                    ? "items.empty.untitledSecret"
                    : "items.empty.untitledNote",
              )
            }}
          </h3>

          <div v-if="item.kind === 'todo' && item.todoEntries.length" class="card-todo-list">
            <button
              v-for="entry in item.todoEntries.slice(0, 4)"
              :key="entry.id"
              type="button"
              :title="t(entry.isCompleted ? 'items.actions.restoreEntry' : 'items.actions.completeEntry')"
              :disabled="item.status === 'archived'"
              @click="toggleTodoEntry(item, entry)"
            >
              <span class="card-todo-check" :class="{ checked: entry.isCompleted }">&#10003;</span>
              <span :class="{ completed: entry.isCompleted }">{{ entry.text }}</span>
            </button>
            <div class="card-todo-progress">
              <span>{{ t("items.todo.progress", { completed: completedTodoCount(item), total: item.todoEntries.length }) }}</span>
              <span v-if="item.todoEntries.length > 4">{{ t("items.todo.more", { count: item.todoEntries.length - 4 }) }}</span>
            </div>
          </div>

          <p v-if="item.kind === 'secret'" class="secret-preview">&bull;&bull;&bull;&bull;&bull;&bull;&bull;&bull;</p>
          <p v-else-if="item.content" class="content-preview">{{ item.content }}</p>
          <p v-else-if="item.kind !== 'todo'" class="content-preview">{{ t("items.empty.note") }}</p>

          <div v-if="item.tags.length" class="tag-list">
            <span v-for="tag in item.tags.slice(0, 3)" :key="tag">#{{ tag }}</span>
          </div>

          <footer>
            <span v-if="item.status === 'archived'">
              {{ t("items.meta.archivedAt", { time: formatTime(item.archivedAt) }) }}
            </span>
            <span v-else-if="item.kind === 'secret'" :class="{ expired: isExpired(item) }">
              {{ isExpired(item) ? t("items.meta.expired") : item.expiresAt ? t("items.meta.expiresAt", { time: formatTime(item.expiresAt) }) : t("items.meta.mcpDenied") }}
            </span>
            <span v-else-if="item.dueAt">{{ t("items.meta.dueAt", { time: formatTime(item.dueAt) }) }}</span>
            <span v-else>{{ formatTime(item.updatedAt) }}</span>
            <button v-if="item.status === 'archived'" :title="t('items.actions.restore')" @click="restoreArchived(item)">
              {{ t("items.actions.restore") }}
            </button>
            <button v-else :title="t('items.actions.archive')" @click="archive(item)">{{ t("items.actions.archive") }}</button>
          </footer>
        </article>
      </section>

      <section v-else class="empty-state">
        <div class="empty-illustration">
          <span></span><span></span><span></span>
        </div>
        <h2>{{ t(filter === "archived" ? "main.archive.emptyTitle" : "main.empty.title") }}</h2>
        <p>{{ t(filter === "archived" ? "main.archive.emptyDescription" : "main.empty.description") }}</p>
        <button v-if="filter !== 'archived'" @click="openCreate()">{{ t("main.empty.action") }}</button>
      </section>
    </main>

    <div v-if="editorOpen" class="modal-backdrop" @mousedown.self="editorOpen = false">
      <form class="editor-modal" @submit.prevent="saveDraft">
        <header>
          <div>
            <p>{{ t(draft.id ? "main.editor.eyebrowEdit" : "main.editor.eyebrowNew") }}</p>
            <h2>{{ t(draft.id ? "main.editor.titleEdit" : "main.editor.titleNew") }}</h2>
          </div>
          <button type="button" :aria-label="t('common.close')" @click="editorOpen = false">×</button>
        </header>

        <div class="kind-switch" :class="{ disabled: draft.id }">
          <button
            v-for="kind in itemKinds"
            :key="kind"
            type="button"
            :class="{ active: draft.kind === kind }"
            :disabled="Boolean(draft.id)"
            @click="selectDraftKind(kind)"
          >
            {{ kindLabel(kind) }}
          </button>
        </div>

        <label>
          <span>{{ t("main.editor.titleLabel") }}</span>
          <input v-model="draft.title" maxlength="160" :placeholder="t('main.editor.titlePlaceholder')" autofocus />
        </label>

        <section v-if="draft.kind === 'todo'" class="todo-editor">
          <header>
            <div>
              <strong>{{ t("main.editor.checklistTitle") }}</strong>
              <span v-if="draft.todoEntries.length">
                {{ draft.todoEntries.filter((entry) => entry.isCompleted).length }}/{{
                  draft.todoEntries.length
                }}
              </span>
            </div>
            <button type="button" :disabled="draft.todoEntries.length >= 200" @click="addDraftTodoEntry">
              {{ t("main.editor.addEntry") }}
            </button>
          </header>
          <div v-if="draft.todoEntries.length" class="todo-editor-list">
            <div v-for="(entry, index) in draft.todoEntries" :key="entry.clientId" class="todo-editor-row">
              <label class="todo-editor-check" :title="t(entry.isCompleted ? 'items.actions.restoreEntry' : 'items.actions.completeEntry')">
                <input v-model="entry.isCompleted" type="checkbox" />
                <span>&#10003;</span>
              </label>
              <input
                v-model="entry.text"
                maxlength="500"
                :class="{ completed: entry.isCompleted }"
                :placeholder="t('main.editor.entryPlaceholder', { index: index + 1 })"
                @keydown.enter.prevent="addDraftTodoEntry"
              />
              <button type="button" class="todo-editor-remove" :title="t('main.editor.removeEntry')" @click="removeDraftTodoEntry(index)">
                ×
              </button>
            </div>
          </div>
          <button v-else type="button" class="todo-editor-empty" @click="addDraftTodoEntry">
            {{ t("main.editor.noEntries") }}
          </button>
        </section>

        <label class="content-field">
          <span>{{ t(draft.kind === "secret" ? "main.editor.secretContentLabel" : draft.kind === "todo" ? "main.editor.todoContentLabel" : "main.editor.contentLabel") }}</span>
          <textarea
            v-model="draft.content"
            :rows="draft.kind === 'todo' ? 4 : 7"
            :placeholder="t(draft.kind === 'todo' ? 'main.editor.todoContentPlaceholder' : 'main.editor.contentPlaceholder')"
          ></textarea>
        </label>

        <div class="form-row">
          <label>
            <span>{{ t("main.editor.groupLabel") }}</span>
            <input
              v-model="draft.group"
              list="item-group-options"
              maxlength="96"
              autocomplete="off"
              spellcheck="false"
              :placeholder="t('main.editor.groupPlaceholder')"
            />
            <datalist id="item-group-options">
              <option v-for="entry in groups" :key="entry.key" :value="entry.key">{{ groupLabel(entry.key) }}</option>
            </datalist>
          </label>
          <label>
            <span>{{ t("main.editor.tagsLabel") }}</span>
            <input v-model="draft.tags" :placeholder="t('main.editor.tagsPlaceholder')" />
          </label>
          <label v-if="draft.kind === 'todo'">
            <span>{{ t("main.editor.dueAtLabel") }}</span>
            <input v-model="draft.dueAt" type="datetime-local" />
          </label>
          <label v-if="draft.kind === 'secret'">
            <span>{{ t("main.editor.expiresAtLabel") }}</span>
            <input v-model="draft.expiresAt" type="datetime-local" />
          </label>
        </div>

        <fieldset class="color-picker">
          <legend>{{ t("main.editor.colorLabel") }}</legend>
          <button
            v-for="color in itemColors"
            :key="color"
            type="button"
            :class="[`swatch-${color}`, { active: draft.color === color }]"
            :title="colorLabel(color)"
            @click="draft.color = color"
          >
            <span></span>
          </button>
        </fieldset>

        <div v-if="draft.kind === 'secret'" class="security-note">
          <span>◇</span>
          <p><strong>{{ t("main.editor.securityLead") }}</strong> {{ t("main.editor.securityBody") }}</p>
        </div>

        <footer>
          <label v-if="!draft.id" class="float-option">
            <input v-model="floatAfterCreate" type="checkbox" />
            <span>{{ t("main.editor.floatAfterCreate") }}</span>
          </label>
          <div>
            <button type="button" class="secondary" @click="editorOpen = false">{{ t("common.cancel") }}</button>
            <button type="submit" class="primary" :disabled="saving">
              {{
                saving
                  ? t("main.editor.saving")
                  : draft.id
                    ? t("main.editor.saveChanges")
                    : draft.kind === "todo"
                      ? t("main.editor.createList")
                      : t("main.editor.createNote")
              }}
            </button>
          </div>
        </footer>
      </form>
    </div>

    <div v-if="mcpPanelOpen" class="modal-backdrop" @mousedown.self="closeMcpPanel">
      <section class="editor-modal mcp-modal" role="dialog" aria-modal="true" aria-labelledby="mcp-panel-title">
        <header>
          <div>
            <p>{{ t("mcp.panel.eyebrow") }}</p>
            <h2 id="mcp-panel-title">{{ t("mcp.panel.title") }}</h2>
          </div>
          <button type="button" :aria-label="t('common.close')" :disabled="mcpSaving" @click="closeMcpPanel">×</button>
        </header>

        <div v-if="mcpError" class="mcp-panel-error" role="alert">
          <span>{{ mcpError }}</span>
          <button type="button" :aria-label="t('common.dismissError')" @click="mcpError = ''">×</button>
        </div>

        <section class="mcp-runtime-card" :class="{ running: mcpStatus?.running, failed: Boolean(mcpStatus?.error) }">
          <span class="mcp-runtime-dot"></span>
          <div>
            <strong>{{ t(mcpStatus?.running ? "mcp.runtime.running" : mcpStatus?.enabled ? "mcp.runtime.notReady" : "mcp.runtime.disabled") }}</strong>
            <code>{{ mcpStatus?.endpoint || "http://127.0.0.1:4784/mcp" }}</code>
            <small>{{ t("mcp.runtime.securityDescription") }}</small>
            <small v-if="mcpStatus?.error" class="mcp-runtime-error">{{ mcpStatus.error }}</small>
          </div>
        </section>

        <section v-if="provisionedClient" class="mcp-token-once" aria-live="polite">
          <header>
            <div>
              <strong>{{ t("mcp.token.title") }}</strong>
              <small>{{ t("mcp.token.description") }}</small>
            </div>
            <button type="button" @click="provisionedClient = null">{{ t("mcp.token.saved") }}</button>
          </header>
          <textarea :value="provisionedClient.bearerToken" readonly spellcheck="false" :aria-label="t('mcp.token.ariaLabel')"></textarea>
          <dl>
            <div><dt>{{ t("mcp.token.endpoint") }}</dt><dd>{{ provisionedClient.endpoint }}</dd></div>
            <div><dt>{{ t("mcp.token.grant") }}</dt><dd>{{ provisionedClient.grant.group }} · {{ roleLabel(provisionedClient.grant.role) }}</dd></div>
            <div><dt>{{ t("mcp.token.groupManagement") }}</dt><dd>{{ t(provisionedClient.client.canManageGroups ? "common.on" : "common.off") }}</dd></div>
          </dl>
        </section>

        <form class="mcp-provision-form" @submit.prevent="provisionMcpClient">
          <header>
            <div>
              <strong>{{ t("mcp.provision.title") }}</strong>
              <small>{{ t("mcp.provision.description") }}</small>
            </div>
          </header>
          <div class="mcp-form-grid">
            <label>
              <span>{{ t("mcp.provision.clientName") }}</span>
              <input v-model="mcpDraft.displayName" maxlength="80" :placeholder="t('mcp.provision.clientPlaceholder')" required />
            </label>
            <label>
              <span>{{ t("mcp.provision.group") }}</span>
              <select v-model="mcpDraft.groupName" required>
                <option v-for="group in groupRecords" :key="group.key" :value="group.key">
                  {{ groupLabel(group.key) }}
                </option>
              </select>
            </label>
            <label>
              <span>{{ t("mcp.provision.permission") }}</span>
              <select v-model="mcpDraft.role">
                <option value="viewer">{{ roleLabel("viewer") }}</option>
                <option value="contributor">{{ roleLabel("contributor") }}</option>
                <option value="coordinator">{{ roleLabel("coordinator") }}</option>
              </select>
            </label>
            <button type="submit" :disabled="mcpSaving">
              {{ t(mcpSaving ? "mcp.provision.creating" : "mcp.provision.create") }}
            </button>
          </div>
          <label class="mcp-capability-option">
            <input v-model="mcpDraft.canManageGroups" type="checkbox" />
            <span>
              <strong>{{ t("mcp.capabilities.manageGroups") }}</strong>
              <small>{{ t("mcp.capabilities.manageGroupsDescription") }}</small>
            </span>
          </label>
          <small v-if="!mcpStatus?.running" class="mcp-form-hint">{{ t("mcp.provision.offlineHint") }}</small>
        </form>

        <form class="mcp-provision-form mcp-grant-form" @submit.prevent="saveMcpGroupGrant">
          <header>
            <div>
              <strong>{{ t("mcp.grants.title") }}</strong>
              <small>{{ t("mcp.grants.description") }}</small>
            </div>
          </header>
          <div v-if="mcpClients.length" class="mcp-form-grid">
            <label>
              <span>{{ t("mcp.grants.client") }}</span>
              <select v-model="mcpGrantDraft.clientId" required @change="syncMcpGrantRole">
                <option v-for="client in mcpClients" :key="client.id" :value="client.id">
                  {{ client.displayName }}
                </option>
              </select>
            </label>
            <label>
              <span>{{ t("mcp.grants.group") }}</span>
              <select v-model="mcpGrantDraft.groupName" required @change="syncMcpGrantRole">
                <option v-for="group in groupRecords" :key="group.key" :value="group.key">
                  {{ groupLabel(group.key) }}
                </option>
              </select>
            </label>
            <label>
              <span>{{ t("mcp.grants.permission") }}</span>
              <select v-model="mcpGrantDraft.role">
                <option value="viewer">{{ roleLabel("viewer") }}</option>
                <option value="contributor">{{ roleLabel("contributor") }}</option>
                <option value="coordinator">{{ roleLabel("coordinator") }}</option>
              </select>
            </label>
            <button type="submit" :disabled="mcpSaving || !mcpGrantDraft.groupName">
              {{ t(selectedMcpGrant ? "mcp.grants.update" : "mcp.grants.add") }}
            </button>
          </div>
          <small v-else class="mcp-form-hint">{{ t("mcp.grants.noClients") }}</small>
        </form>

        <section class="mcp-client-section">
          <header>
            <div>
              <strong>{{ t("mcp.clients.title") }}</strong>
              <small>{{ t("mcp.clients.count", { count: mcpClients.length }) }}</small>
            </div>
          </header>

          <div v-if="mcpLoading" class="mcp-panel-loading">{{ t("mcp.clients.loading") }}</div>
          <div v-else-if="mcpClients.length" class="mcp-client-list">
            <article v-for="client in mcpClients" :key="client.id" class="mcp-client-card">
              <header>
                <div>
                  <strong>{{ client.displayName }}</strong>
                  <small>{{ transportLabel(client.transport) }}</small>
                </div>
                <button type="button" :disabled="mcpSaving" @click="revokeMcpClient(client)">{{ t("mcp.clients.revokeClient") }}</button>
              </header>
              <p>{{ t("mcp.clients.timestamps", { createdAt: formatMcpTime(client.createdAt), lastUsedAt: formatMcpTime(client.lastUsedAt) }) }}</p>
              <label class="mcp-client-capability">
                <input
                  type="checkbox"
                  :checked="client.canManageGroups"
                  :disabled="mcpSaving"
                  @change="changeMcpGroupManagement(client, $event)"
                />
                <span>
                  <strong>{{ t("mcp.capabilities.manageGroups") }}</strong>
                  <small>{{ t("mcp.capabilities.manageGroupsCardDescription") }}</small>
                </span>
              </label>
              <div v-if="grantsForClient(client.id).length" class="mcp-grant-list">
                <div v-for="grant in grantsForClient(client.id)" :key="`${grant.clientId}:${grant.group}`">
                  <span><strong># {{ grant.group }}</strong><small>{{ roleLabel(grant.role) }}</small></span>
                  <span class="mcp-grant-actions">
                    <button type="button" :disabled="mcpSaving" @click="editMcpGrant(grant)">{{ t("mcp.clients.editGrant") }}</button>
                    <button type="button" :disabled="mcpSaving" @click="revokeMcpGrant(grant)">{{ t("mcp.clients.revokeGrant") }}</button>
                  </span>
                </div>
              </div>
              <p v-else class="mcp-no-grants">{{ t("mcp.clients.noGrants") }}</p>
            </article>
          </div>
          <div v-else class="mcp-panel-empty">
            {{ t("mcp.clients.empty") }}
          </div>
        </section>
      </section>
    </div>

    <Transition name="toast">
      <div v-if="toast" class="toast">{{ toast }}</div>
    </Transition>
  </div>
</template>
