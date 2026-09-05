# Tips MCP contract (v0)

This file records the implemented MCP v1 core and keeps later multi-agent coordination work explicit. Items and TODO forms are shared application data; desktop window state is not part of this protocol.

## 1. Domain boundaries

- `group` is a persistent task-space entity, not a free-form tag. Empty groups remain in `item_groups`; UI-created items default to `inbox`, while MCP item creates must pass an existing authorized group explicitly.
- Group keys are trimmed, lower-cased, limited to 96 characters, and may contain Unicode letters/numbers plus `-`, `_`, `.`, and `/`. Empty path segments, `.` and `..` are rejected.
- `tags` remain multi-valued descriptive metadata and must not be used for authorization, assignment, or ownership.
- `kind = todo` is the shared task form. Notes may be shared as context. Secrets remain `mcp_access = deny` regardless of group.
- `revision` is the item optimistic-lock token. Checklist entries keep their own stable ID and revision.

## 2. Client identity and group access

- Every network client maps to one `mcp_clients.id`; raw bearer tokens are shown once and only their hashes are stored.
- The trusted main-window administration surface can add or update multiple group grants for an existing client without rotating its bearer token. Sticky windows and MCP callers cannot invoke credential or grant administration commands.
- Actor identity is derived from each authenticated HTTP request. A tool caller cannot supply `actorClientId`, `claimedBy`, or an equivalent identity field.
- `mcp_group_grants(client_id, group_key, role)` grants `viewer`, `contributor`, or `coordinator` access per group. Viewer is read-only; contributor and coordinator can mutate items in authorized groups.
- Group creation additionally requires the client-level `groups:manage` capability, which is off by default and can only be changed in the trusted main-window administration surface. Creating a group grants that explicitly authorized caller coordinator access to the new group. Renaming requires both `groups:manage` and coordinator access to the source group.
- Moving an item requires write access in both its current group and its target group. This prevents a client from using a move as an unauthorized read or write bridge.
- Revoked clients cannot read, mutate, renew a claim, or receive change events.

## 3. Assignment versus claim

These are planned coordination concepts, deliberately separate from the implemented v1 content/status tools, and only apply to TODO items:

- Assignment is durable routing: `assignedToClientId` says who should work on the task. Only a coordinator (or the current assignee where policy permits) changes it.
- Claim is a short lease: `claimedByClientId`, `claimExpiresAt`, and `claimRevision` say who is actively working on it.
- Claim, renewal, and release are atomic compare-and-set operations. An unexpired claim held by another client must fail with a structured conflict result.
- A claim has a bounded lease and must be renewed. This prevents a crashed terminal from owning a task forever.
- Updating task content still requires `expectedRevision`; claim mutation requires `expectedClaimRevision` after the initial claim.

## 4. MCP surface

Implemented v1 read tools:

- `tips_groups_list()`
- `tips_items_list(group, kind?, status?, updatedAfter?, limit?)`
- `tips_item_get(id)`

Implemented v1 mutation tools:

- `tips_group_create(group)` — requires explicit client capability `groups:manage`
- `tips_group_rename(group, newGroup)` — requires `groups:manage` plus source-group `coordinator`
- `tips_item_create(group, kind, title, content, tags?, dueAt?, todoEntries?)`
- `tips_item_update(id, expectedRevision, title?, content?, tags?, dueAt?, clearDueAt?, replaceTodoEntries?)`
- `tips_item_move(id, expectedRevision, targetGroup)`
- `tips_item_set_status(id, expectedRevision, status)`
- `tips_todo_entry_set_completed(itemId, entryId, expectedItemRevision, expectedEntryRevision, isCompleted)`

`tips_item_update` is a patch and deliberately cannot move an item between groups; use the separately authorized `tips_item_move` operation. Omitted scalar fields preserve their current values; `clearDueAt = true` removes a due date. For TODO items, supplying `replaceTodoEntries` performs the authorized, revision-checked complete checklist replacement; omitting it leaves the checklist unchanged. Group rename atomically migrates all items, historical item events, and active or revoked grants; it increments affected item revisions, and `inbox` cannot be renamed.

Planned coordination/change tools:

- `tips_changes(group, afterEventId, limit?)`
- `tips_task_assign(id, expectedRevision, assigneeClientId?)`
- `tips_task_claim(id, leaseSeconds)`
- `tips_task_renew_claim(id, expectedClaimRevision, leaseSeconds)`
- `tips_task_release(id, expectedClaimRevision)`

Implemented tool results return the normalized MCP item plus its current revisions. They contain no window position, size, pin, opacity, color, theme, sensitivity, encryption, or other desktop-private fields. Operational failures and revision conflicts are structured tool errors rather than silent last-write-wins behavior.

Resources such as `tips://groups/{group}/tasks` are not exposed in v1. The domain TODO board is not represented by MCP's optional Tasks extension: that extension tracks asynchronous execution of an MCP request, while Tips TODO items are application data.

## 5. Change stream

Migrations 005 and 006 add:

- `item_groups(group_key, created_at, updated_at)`
- `task_events(event_id, item_id, group_key, actor_client_id, action, item_revision, payload_json, created_at)`
- `mcp_group_grants(client_id, group_key, role, created_at, revoked_at)`

Every implemented MCP item mutation appends its event in the same SQLite transaction as the data change; an item move records both source and target groups in its payload. Group creation and rename are atomic, but need a dedicated group-event stream before they can participate in `tips_changes`. `tips_changes` is not exposed yet; before it is added, desktop-UI mutations must also append compatible events. Assignment and leases will require a later `task_coordination(item_id, assigned_client_id, claimed_client_id, claim_expires_at, claim_revision, updated_at)` migration.

## 6. Transport and exposure

- Implemented default: one Streamable HTTP endpoint at `http://127.0.0.1:4784/mcp`; every request requires a transport-bound Bearer token and active group grant.
- The implementation pins the official Rust SDK `rmcp 3.1.2`, targets protocol `2026-07-28`, uses stateless requests, validates loopback Host/Origin values, and caps request bodies at 1 MiB.
- The current server refuses LAN/non-loopback configuration. There is no unauthenticated fallback and binding `0.0.0.0` is never an implicit consequence of enabling MCP.
- A later stdio bridge may connect clients that cannot use Streamable HTTP; it must use the same authorization and store boundary.
- Future LAN mode is explicit opt-in and requires a bearer token, per-group grants, Origin validation, and TLS directly or through a trusted reverse proxy.
