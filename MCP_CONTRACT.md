# Tips MCP contract (draft)

This file freezes the application-level contract before the MCP listener is implemented. The server is not running yet.

## 1. Domain boundaries

- `group` is a stable task-space key, not a free-form tag. UI-created items default to `inbox`; MCP creates must pass it explicitly.
- Group keys are trimmed, lower-cased, limited to 96 characters, and may contain Unicode letters/numbers plus `-`, `_`, `.`, and `/`. Empty path segments, `.` and `..` are rejected.
- `tags` remain multi-valued descriptive metadata and must not be used for authorization, assignment, or ownership.
- `kind = todo` is the shared task form. Notes may be shared as context. Secrets remain `mcp_access = deny` regardless of group.
- `revision` is the item optimistic-lock token. Checklist entries keep their own stable ID and revision.

## 2. Client identity and group access

- Every network client maps to one `mcp_clients.id`; raw bearer tokens are shown once and only their hashes are stored.
- Actor identity is derived from the authenticated connection. A tool caller cannot supply `actorClientId`, `claimedBy`, or an equivalent identity field.
- A later `mcp_group_grants(client_id, group_key, role)` table will grant `viewer`, `contributor`, or `coordinator` access per group.
- Revoked clients cannot read, mutate, renew a claim, or receive change events.

## 3. Assignment versus claim

These are deliberately separate concepts and only apply to TODO items:

- Assignment is durable routing: `assignedToClientId` says who should work on the task. Only a coordinator (or the current assignee where policy permits) changes it.
- Claim is a short lease: `claimedByClientId`, `claimExpiresAt`, and `claimRevision` say who is actively working on it.
- Claim, renewal, and release are atomic compare-and-set operations. An unexpired claim held by another client must fail with a structured conflict result.
- A claim has a bounded lease and must be renewed. This prevents a crashed terminal from owning a task forever.
- Updating task content still requires `expectedRevision`; claim mutation requires `expectedClaimRevision` after the initial claim.

## 4. Initial MCP surface

Read tools:

- `tips_groups_list()`
- `tips_items_list(group, kind?, status?, updatedAfter?, cursor?)`
- `tips_item_get(id)`
- `tips_changes(group, afterEventId, limit?)`

Mutation tools:

- `tips_item_create(group, kind, title, content, tags?, dueAt?, todoEntries?)`
- `tips_item_update(id, expectedRevision, group, title, content, tags?, dueAt?, todoEntries?)`
- `tips_item_set_status(id, expectedRevision, status)`
- `tips_todo_entry_add/update/set_completed/delete(...)`
- `tips_task_assign(id, expectedRevision, assigneeClientId?)`
- `tips_task_claim(id, leaseSeconds)`
- `tips_task_renew_claim(id, expectedClaimRevision, leaseSeconds)`
- `tips_task_release(id, expectedClaimRevision)`

Tool results return the full normalized item plus its current revisions. Conflicts are data, not silent last-write-wins behavior.

The server will also expose group/task resources such as `tips://groups/{group}/tasks`. The domain TODO board is not represented by MCP's optional Tasks extension: that extension tracks asynchronous execution of an MCP request, while Tips TODO items are application data.

## 5. Change stream

Coordination needs an append-only event cursor rather than polling the whole database. The next migration should add:

- `task_events(event_id, item_id, group_key, actor_client_id, action, item_revision, payload_json, created_at)`
- `task_coordination(item_id, assigned_client_id, claimed_client_id, claim_expires_at, claim_revision, updated_at)`
- `mcp_group_grants(client_id, group_key, role, created_at, revoked_at)`

`tips_changes` reads events strictly after `afterEventId`. Every mutation appends its event in the same SQLite transaction as the data change.

## 6. Transport and exposure

- Default: one Streamable HTTP endpoint bound to `127.0.0.1`; no LAN listener and no unauthenticated fallback.
- Compatibility target: the current `2026-07-28` MCP protocol, with `2025-11-25` compatibility only where the selected SDK can test it reliably.
- A small stdio bridge may connect clients that cannot use Streamable HTTP; it talks to the same desktop-owned service and database.
- LAN mode is explicit opt-in and requires a bearer token, per-group grants, Origin validation, and TLS (directly or through a trusted reverse proxy). Binding `0.0.0.0` is never an implicit consequence of enabling MCP.
