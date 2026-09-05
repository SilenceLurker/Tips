export type ItemKind = "note" | "todo" | "secret";
export type ItemStatus = "active" | "completed" | "archived";
export type Sensitivity = "standard" | "sensitive";
export type McpAccess = "inherit" | "deny" | "read" | "read_write";
export type ItemColor = "sun" | "mint" | "sky" | "rose" | "lilac" | "paper";
export type ThemeMode = "light" | "black_gold";
export type LocalePreference = "system" | "zh-CN" | "en-US";
export type SupportedLocale = Exclude<LocalePreference, "system">;
export type McpTransport = "stdio" | "loopback_http" | "lan_http";
export type McpGroupRole = "viewer" | "contributor" | "coordinator";

export interface McpRuntimeStatus {
  enabled: boolean;
  running: boolean;
  endpoint: string;
  error: string | null;
}

export interface McpClient {
  id: string;
  displayName: string;
  transport: McpTransport;
  canManageGroups: boolean;
  createdAt: number;
  lastUsedAt: number | null;
  revokedAt: number | null;
}

export interface McpGroupGrant {
  clientId: string;
  group: string;
  role: McpGroupRole;
  createdAt: number;
  revokedAt: number | null;
}

export interface ProvisionMcpClientInput {
  displayName: string;
  groupName: string;
  role: McpGroupRole;
  canManageGroups: boolean;
}

export interface ProvisionedMcpClient {
  client: McpClient;
  bearerToken: string;
  grant: McpGroupGrant;
  endpoint: string;
}

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

export interface ItemGroup {
  key: string;
  itemCount: number;
  visibleItemCount: number;
  createdAt: number;
  updatedAt: number;
}

export interface GroupChangedEvent {
  previousKey: string | null;
  group: ItemGroup;
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

export const colorOptions: ItemColor[] = ["sun", "mint", "sky", "rose", "lilac", "paper"];
export const kindOptions: ItemKind[] = ["note", "todo", "secret"];
