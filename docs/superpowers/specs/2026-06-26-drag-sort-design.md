# Drag-to-Reorder & Drag-to-Regroup — Design Spec

**Date:** 2026-06-26
**Status:** Approved
**Goal:** Let the user drag project entries to reorder them within a group and to move them across groups. Manual order persists across rescans (lives in `prefs.db`, not the read-only `index.db`).

## Context & Constraints

- `index.db` is **read-only** — `sessionatlas scan` recreates it. Any manual sort order MUST live in `prefs.db` (which already holds `project_groups` / `project_group_assignments` and survives rescans).
- The server returns projects sorted by `last_accessed_at DESC`. The frontend currently re-buckets by group at render time but preserves server order within a bucket. Per-bucket reordering will happen at render time too.
- A separate browser-demo (`SAMPLE`) mode must keep working (no Tauri commands available there).

## Decisions (settled in brainstorming)

1. **Per-group ordering** — each group maintains its own manual sequence. Order field is conceptually `(group_key, sort_order)`.
2. **First reorder-drag locks that group to manual order.** Untouched groups stay recency-sorted. A newly scanned project landing in a manual group auto-appends (no sort row → sorts to end). No rescan-sync step, no race.
3. **Drag handles both in-group reorder and cross-group move.** Dropping between rows of group B reorders within B (and moves the project into B if it came from elsewhere). Dropping on a group header moves the project into that group (appends if target is manual, else recency — does not lock).

## Data Model (prefs.db)

New table:

```sql
CREATE TABLE project_sort (
  project_id  TEXT    PRIMARY KEY,
  group_key   TEXT    NOT NULL,    -- "ungrouped" | str(group_id)
  sort_order  INTEGER NOT NULL
);
CREATE INDEX idx_project_sort_group ON project_sort(group_key, sort_order);
```

**Manual-ness is derived, not stored:** a group is "manual" iff ≥1 of its members has a row in `project_sort`. No separate flag table; the ungrouped bucket is handled uniformly via `group_key = "ungrouped"`.

**Missing sort row = end of list:** when sorting a manual group, `sortOrders[p.id] ?? Infinity`. A newly scanned project in a manual group therefore auto-appends without a row being written on rescan.

## Backend (src-tauri/src/lib.rs)

Add the table to the `open_prefs_db` `execute_batch` (idempotent `CREATE TABLE IF NOT EXISTS`).

New commands (registered in `run()`):

- **`list_sort_orders()`** → `Vec<{projectId, groupKey, sortOrder}>` (serde camelCase). Frontend loads this alongside groups/assignments in `loadGroups`.

- **`set_group_order(group_key: String, ordered_ids: Vec<String>)`** — the single drag command. Rewrites `sort_order = index*10` for `ordered_ids`, sets their `group_key`, AND upserts `project_group_assignments` so cross-group positional moves update assignment too (ungrouped → assignment row deleted). Locks the target group (gives every listed member a row). Implementation:
  - Delete rows for `ordered_ids` (in case they were in other groups) — `DELETE FROM project_sort WHERE project_id IN (...)`.
  - Insert each `(project_id, group_key, i*10)`.
  - Reconcile `project_group_assignments`: for group_key `"ungrouped"` delete the assignment row; else upsert `group_id = int(group_key)`.
  - Single `with_prefs` transaction (closure does all SQL under the lock).

- **Enhance `assign_project_to_group`** to reconcile sort so the existing dropdown/header-drop path stays consistent:
  - After setting group_id, if target group is manual → upsert `project_sort` with `(group_key=str(gid), sort_order = max+10)` (appends with a position).
  - If target group not manual → delete the project's `project_sort` row (stays recency, does not lock).
  - Ungrouped (`group_id=None`): always delete the sort row (ungrouped is recency by default; dragging onto the ungrouped header is the explicit way to append-position it, which goes through `set_group_order` instead).

- **`delete_group`** already cascades `project_group_assignments` via FK. Add: also `DELETE FROM project_sort WHERE group_key = str(group_id)` so members fall back to ungrouped recency cleanly (orphaned sort rows in a deleted group are harmless — `Infinity`-fallback ignores group_key mismatch — but cleaning is tidy).

## Frontend (frontend/app.js)

### State & loading

- `state.sortOrders = {}` — `{projectId: sortOrder}` (flattened from `list_sort_orders`).
- Loaded in `loadGroups` via `Promise.all([list_groups, list_group_assignments, list_sort_orders])`.
- Browser-demo branch: seed `state.sortOrders = {}` (manual order only exists after a drag in real mode; demo just shows recency).

### Render-time bucket sort

- Derive `manualGroups` = set of group keys where any `state.all` project with that key has a `sortOrders` entry.
- In `renderLedger`, after bucketing `visible` projects by group, sort each bucket:
  - manual group → `(sortOrders[p.id] ?? Infinity)` ascending, ties broken by `lastAccessedAt` DESC.
  - non-manual → `lastAccessedAt` DESC (unchanged).
- `state.filtered` (the nav/count set) still excludes collapsed groups and still reflects the render order (cursor moves follow the visible rendered order).

### Drag interaction

- `.entry` rows get `draggable="true"`.
- `dragstart` on `.entry`: store `state._dragId = project.id`, add `.is-dragging` to source.
- `dragover` on `.entry`: `e.preventDefault()` (allow drop); compute insertion index relative to pointer Y (top half → before, bottom half → after); show a 2px drop-indicator line via a `.drop-before`/`.drop-after` class on the row.
- `dragover` on `.ledger__group` (header): `e.preventDefault()`; add `.is-drop-target` (move-into-group).
- `drop` on `.entry`: build the new ordered id list for the **target group** (the group of the row under the pointer), splicing the dragged project in at the insertion index, removing it from its old position. Call `set_group_order(targetKey, newList)`. (This handles both in-group reorder and cross-group positional move in one path.)
- `drop` on `.ledger__group`: call `assign_project_to_group(p, G)` where G is the header's group (ungrouped → null). Header-drop = move-into-group (appends if target manual, else recency, does not lock target).
- `dragend`: clear `.is-dragging`, drop indicators, `state._dragId`.

### Optimistic update (matches existing `setProjectGroup` pattern)

- Mutate `state.sortOrders` / `state.assignments` to the expected post-drop state, re-render immediately.
- Await the command; on failure → `reload()` + `showError` (full re-fetch is the safe revert since sort state is server-authoritative).

### CSS (styles.css)

- `.entry.is-dragging { opacity: .4; }`
- `.entry.drop-before { box-shadow: inset 0 2px 0 var(--bone-accent); }`
- `.entry.drop-after { box-shadow: inset 0 -2px 0 var(--bone-accent); }`
- `.ledger__group.is-drop-target { background: rgba(...); outline: ...; }`
- Use existing design tokens (no new primitives) per CLAUDE.md.

## Edge Cases

- **Collapsed group:** drop targets are only visible (expanded) rows + all headers. No interaction with the collapse-empty bug.
- **Cross-group positional move** updates assignment in the single `set_group_order` call (assignment + sort in one transaction).
- **New project in a manual group on rescan** → no sort row → sorts to end (Infinity) automatically. No sync step.
- **Group deletion** clears members' sort rows → they return to ungrouped recency.
- **Browser demo mode** (no Tauri): drag is wired but the command is a no-op / local-only reorder so the demo still illustrates the interaction. Keep it simple — optimistic local reorder without persistence.

## Testing

No test suite exists. Verification plan:
- `cd src-tauri && cargo check` — Rust compiles.
- `node --check frontend/app.js` — JS parses.
- Manual: run `cargo tauri dev`, drag a project within its group (order changes + persists across reload), drag across groups (assignment updates + position), drop on header (move-into-group), confirm a freshly rescanned project in a manual group lands at the end, confirm an untouched group stays recency-sorted.

## Out of Scope (YAGNI)

- No "reset group to recency" action (can add a menu item later).
- No multi-select drag.
- No drag-between-windows.
