export type ItemKind = "note" | "todo" | "secret";
export type ItemStatus = "active" | "completed" | "archived";
export type Sensitivity = "standard" | "sensitive";
export type McpAccess = "inherit" | "deny" | "read" | "read_write";
export type ItemColor = "sun" | "mint" | "sky" | "rose" | "lilac" | "paper";
export type ThemeMode = "light" | "black_gold";

export interface WindowState {
  x: number | null;
  y: number | null;
  width: number;
  height: number;
  monitorName: string | null;
  scaleFactor: number;
  alwaysOnTop: boolean;
  backgroundOpacity: number;
  textOpacity: number;
  isVisible: boolean;
  updatedAt: number;
}

export interface TodoEntry {
  id: string;
  itemId: string;
  text: string;
  isCompleted: boolean;
  position: number;
  revision: number;
  createdAt: number;
  updatedAt: number;
  completedAt: number | null;
}

export interface Item {
  id: string;
  kind: ItemKind;
  group: string;
  title: string;
  content: string;
  status: ItemStatus;
  sensitivity: Sensitivity;
  color: ItemColor;
  tags: string[];
  dueAt: number | null;
  expiresAt: number | null;
  completedAt: number | null;
  mcpAccess: McpAccess;
  revision: number;
  createdAt: number;
  updatedAt: number;
  archivedAt: number | null;
  window: WindowState | null;
  todoEntries: TodoEntry[];
}

export interface CreateItemInput {
  kind: ItemKind;
  group: string;
  title: string;
  content: string;
  color: ItemColor;
  tags: string[];
  dueAt: number | null;
  expiresAt: number | null;
  mcpAccess: McpAccess;
  todoEntries: string[];
}

export interface TodoEntryDraftInput {
  id: string | null;
  expectedRevision: number | null;
  text: string;
  isCompleted: boolean;
}

export interface UpdateItemInput {
  id: string;
  expectedRevision: number;
  group: string;
  title: string;
  content: string;
  color: ItemColor;
  tags: string[];
  dueAt: number | null;
  expiresAt: number | null;
  todoEntries?: TodoEntryDraftInput[];
}

export const colorOptions: Array<{ value: ItemColor; label: string }> = [
  { value: "sun", label: "暖黄" },
  { value: "mint", label: "薄荷" },
  { value: "sky", label: "雾蓝" },
  { value: "rose", label: "浅绯" },
  { value: "lilac", label: "丁香" },
  { value: "paper", label: "纸白" },
];

export const kindLabels: Record<ItemKind, string> = {
  note: "便签",
  todo: "TODO",
  secret: "临时秘密",
};
