import { invoke } from "@tauri-apps/api/core";
import type {
  CreateItemInput,
  ItemGroup,
  Item,
  ItemStatus,
  LocalePreference,
  McpClient,
  McpGroupGrant,
  McpRuntimeStatus,
  ProvisionMcpClientInput,
  ProvisionedMcpClient,
  ThemeMode,
  TodoEntry,
  UpdateItemInput,
  WindowState,
} from "./types";

export const groupsApi = {
  list() {
    return invoke<ItemGroup[]>("list_groups");
  },

  create(group: string) {
    return invoke<ItemGroup>("create_group", { input: { group } });
  },

  rename(group: string, newGroup: string) {
    return invoke<ItemGroup>("rename_group", { input: { group, newGroup } });
  },
};

export const itemsApi = {
  list(includeArchived = false) {
    return invoke<Item[]>("list_items", { includeArchived });
  },

  get(id: string) {
    return invoke<Item>("get_item", { id });
  },

  create(input: CreateItemInput) {
    return invoke<Item>("create_item", { input });
  },

  update(input: UpdateItemInput) {
    return invoke<Item>("update_item", { input });
  },

  setStatus(item: Item, status: ItemStatus) {
    return invoke<Item>("set_item_status", {
      input: {
        id: item.id,
        expectedRevision: item.revision,
        status,
      },
    });
  },

  addTodoEntry(item: Item, text: string) {
    return invoke<Item>("add_todo_entry", {
      input: {
        itemId: item.id,
        expectedItemRevision: item.revision,
        text,
      },
    });
  },

  updateTodoEntry(item: Item, entry: TodoEntry, text: string) {
    return invoke<Item>("update_todo_entry", {
      input: {
        itemId: item.id,
        entryId: entry.id,
        expectedItemRevision: item.revision,
        expectedEntryRevision: entry.revision,
        text,
      },
    });
  },

  setTodoEntryCompleted(item: Item, entry: TodoEntry, isCompleted: boolean) {
    return invoke<Item>("set_todo_entry_completed", {
      input: {
        itemId: item.id,
        entryId: entry.id,
        expectedItemRevision: item.revision,
        expectedEntryRevision: entry.revision,
        isCompleted,
      },
    });
  },

  deleteTodoEntry(item: Item, entry: TodoEntry) {
    return invoke<Item>("delete_todo_entry", {
      input: {
        itemId: item.id,
        entryId: entry.id,
        expectedItemRevision: item.revision,
        expectedEntryRevision: entry.revision,
      },
    });
  },

  openSticky(itemId: string) {
    return invoke<string>("open_sticky_window", { itemId });
  },

  hideSticky(itemId: string) {
    return invoke<void>("hide_sticky_window", { itemId });
  },

  setStickyPin(itemId: string, pinned: boolean) {
    return invoke<WindowState>("set_sticky_pin", { itemId, pinned });
  },

  setStickyOpacity(
    itemId: string,
    backgroundOpacity: number,
    textOpacity: number,
  ) {
    return invoke<WindowState>("set_sticky_opacity", {
      input: { itemId, backgroundOpacity, textOpacity },
    });
  },
};

export const settingsApi = {
  getTheme() {
    return invoke<ThemeMode>("get_theme");
  },

  setTheme(theme: ThemeMode) {
    return invoke<ThemeMode>("set_theme", { theme });
  },

  getLocale() {
    return invoke<LocalePreference>("get_locale");
  },

  setLocale(locale: LocalePreference) {
    return invoke<LocalePreference>("set_locale", { locale });
  },
};

export const mcpApi = {
  getRuntimeStatus() {
    return invoke<McpRuntimeStatus>("get_mcp_runtime_status");
  },

  listClients() {
    return invoke<McpClient[]>("list_mcp_clients");
  },

  listGroupGrants() {
    return invoke<McpGroupGrant[]>("list_mcp_group_grants");
  },

  setGroupGrant(clientId: string, groupName: string, role: McpGroupGrant["role"]) {
    return invoke<McpGroupGrant>("set_mcp_group_grant", {
      clientId,
      groupName,
      role,
    });
  },

  setGroupManagement(clientId: string, allowed: boolean) {
    return invoke<void>("set_mcp_group_management", { clientId, allowed });
  },

  provisionClient(input: ProvisionMcpClientInput) {
    return invoke<ProvisionedMcpClient>("provision_mcp_client", {
      displayName: input.displayName,
      groupName: input.groupName,
      role: input.role,
      canManageGroups: input.canManageGroups,
    });
  },

  revokeClient(clientId: string) {
    return invoke<void>("revoke_mcp_client", { clientId });
  },

  revokeGroupGrant(clientId: string, groupName: string) {
    return invoke<void>("revoke_mcp_group_grant", { clientId, groupName });
  },
};

export function errorMessage(error: unknown, fallback = "An unknown error occurred"): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return fallback;
}
