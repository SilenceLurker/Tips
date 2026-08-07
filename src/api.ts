import { invoke } from "@tauri-apps/api/core";
import type {
  CreateItemInput,
  Item,
  ItemStatus,
  ThemeMode,
  TodoEntry,
  UpdateItemInput,
  WindowState,
} from "./types";

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
};

export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "发生了未知错误";
}
