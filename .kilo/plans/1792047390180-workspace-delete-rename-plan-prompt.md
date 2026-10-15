# Plan — Workspace rename and deletion

Implementation plan for the feature blurb: name workspaces, delete a workspace behind a
GitHub-style typed confirmation, and move both actions to the workspace list (the picker),
outside an open workspace.

MODE: implementation. This file supersedes the earlier stage-1 plan prompt.

## Goal

- Rename a workspace from the picker.
- Delete a workspace from the picker, behind a two-step confirm ("Are you sure?" then type the
  workspace name), matching GitHub's repository delete flow.
- Keep workspace management out of an open workspace: inside a workspace, only forecasting
  controls remain; rename/delete live on `WorkspacePicker`.

## Fixed decisions (defaults from the plan prompt, now baked in)

1. Name source of truth = the workspace folder name. Rename moves the directory on disk and
   updates the recent-workspaces entry. No stored display-name field.
2. Delete removes the entire workspace directory (`workspace.db` plus `staging/`, `sources/`,
   `recovery/`, `jobs/`, `exports/`, `backups/`). It is permanent; no OS trash and no new
   dependency (repo forbids new dependencies for this).
3. The default application-data workspace (`default_workspace_path`, app-data `/workspace`)
   cannot be renamed or deleted. Enforced in Rust and hidden in the UI.
4. Both actions are refused while an import or forecast job is active (`ensure_idle` /
   `workspace_busy`), and refused for a workspace that is currently open.
5. Typed confirmation is an exact match of the workspace's current name.
6. User-facing copy is English, matching the rest of the UI (`src/lib/frontend/i18n.ts:1`);
   Rust error messages stay English with stable codes.

## Out of scope

- Deleting forecast data, import revisions, or runs.
- Trash / undo / restore of a deleted workspace (ADR-0008's verified-backup gate is
  unimplemented).
- Any schema or migration change.
- New dependency, unrelated refactoring.
- Localizing new copy beyond English.

## Tasks (ordered)

### 1. Rust core — rename

- Add `workspace_rename(path: String, name: String) -> AppResult<WorkspaceRef>` in
  `src-tauri/src/desktop.rs` (next to `workspaces_forget`).
- Validate the new name: trimmed, non-empty, no `/`, no `\`, not `.` or `..`, no NUL, sane
  length. Reject with a new stable code (`invalid_workspace_name`).
- Compute `target = parent(path).join(name)`. Reject if `target` already exists
  (`workspace_name_taken`).
- Guard: `ensure_idle()`; reject if `path` equals the currently open workspace path
  (`workspace_busy`) or the default workspace path (`workspace_default`).
- `std::fs::rename(path, target)`, then update the registry (`record` with the new path) and
  return the updated `WorkspaceRef`.

### 2. Rust core — delete

- Add `workspace_delete(path: String, confirm_name: String) -> AppResult<()>` in
  `src-tauri/src/desktop.rs`.
- Guards in order: `ensure_idle()`; reject default workspace (`workspace_default`); reject the
  currently open workspace (`workspace_busy`); require the workspace database to exist
  (`workspace_not_found`).
- Verify `confirm_name` equals the current folder name exactly (`confirm_name_mismatch`); derive
  the expected name from the path, not from the caller.
- `std::fs::remove_dir_all(path)`, then `recent::forget(entries, &path)` and save the registry.
- Do not touch anything outside the workspace directory.

### 3. Registry helper

- Add `recent::rename(entries, old_path, new_path)` in `src-tauri/src/recent.rs` that re-points an
  existing entry's `path` and `name` (or, if absent, is a no-op), keeping `last_opened_at`.
- Unit-test it beside the existing `recent.rs` tests.

### 4. Register commands and regenerate bindings

- Add `workspace_rename`, `workspace_delete` to `collect_commands![...]` in
  `command_builder()` (`src-tauri/src/desktop.rs:721`).
- Regenerate `src/lib/bindings.ts` (the `export_generated_frontend_bindings` test at
  `src-tauri/src/desktop.rs:790` already writes it).

### 5. Frontend IPC wrappers

- Add `renameWorkspace(path, name)` and `deleteWorkspace(path, confirmName)` to
  `src/lib/frontend/ipc.ts`, unwrapping the Result like the existing wrappers.

### 6. Frontend helpers

- Extend `src/lib/frontend/home.ts`:
  - `canManageWorkspace(entry)` — true when `entry.exists && !entry.is_default`.
  - `confirmWorkspaceName(typed, name)` — exact-match boolean.
  - extend `createWorkspaceHint` (or add a `workspaceActionHint`) for `invalid_workspace_name`,
    `workspace_name_taken`, `confirm_name_mismatch`, `workspace_default`.

### 7. Picker UI

- `src/lib/components/WorkspacePicker.svelte`: for each row where `canManageWorkspace(entry)`,
  add **Rename** and **Delete** buttons.
  - Rename: inline input prefilled with the current name; Confirm disabled until valid.
  - Delete: two steps — first an "Are you sure?" confirmation, then a text input that must
    exactly match the name; the destructive Confirm stays disabled until it matches. Show the
    irreversibility warning plainly here (this is a destructive, irreversible action).
- Add matching props/handlers; wire them in `src/routes/+page.svelte` next to
  `handleForgetWorkspace`.
- After either action, invalidate the `['workspaces']` query.

### 8. Keep management out of the open workspace

- `src/lib/components/WorkspacePanel.svelte`: keep forecast/import/weekly controls. "Switch
  workspace…" stays as navigation back to the picker; do not add rename/delete here.

### 9. Docs and ADR

- Add `docs/architecture/adr/0024-workspace-rename-and-deletion.md`: rename = folder move, delete
  = permanent directory removal, default workspace exempt, busy/open guards, and the explicit
  note that ADR-0008's verified-backup gate is unimplemented so delete is irreversible.
- Update `docs/architecture/contracts/TAURI_IPC.md` with the two new commands and their error
  codes.
- Add a one-line **Workspace name** entry to `GLOSSARY.md` if the folder-name rule is not already
  obvious there.

### 10. Tests

- Rust (`src-tauri/src/recent.rs`): `recent::rename` re-points path and name.
- Rust (`src-tauri/src/desktop.rs` tests): name validation rejects separators/empty/dot names;
  `confirm_name` mismatch refused; default workspace refused. Use `tempfile` for any filesystem
  cases; keep DB-open cases out (pure path + validation logic only).
- Frontend (`tests/frontend/home.test.ts`): `canManageWorkspace` false for default and
  non-existent rows; `confirmWorkspaceName` is exact/case-sensitive; new hint codes map to copy.

## Files touched

- `src-tauri/src/desktop.rs` (commands, registration, tests)
- `src-tauri/src/recent.rs` (rename helper, tests)
- `src/lib/bindings.ts` (generated)
- `src/lib/frontend/ipc.ts`, `src/lib/frontend/home.ts`
- `src/lib/components/WorkspacePicker.svelte`, `src/routes/+page.svelte`
- `tests/frontend/home.test.ts`
- `docs/architecture/contracts/TAURI_IPC.md`, `docs/architecture/adr/0024-...`, `GLOSSARY.md`

## Validation

Run from the repo root:

- `cargo test --manifest-path src-tauri/Cargo.toml`
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
- `npm run check`, `npm test`, `npm run build`

Manual check in the desktop shell: rename a workspace and confirm the folder and list update;
delete a workspace and confirm the directory is gone and the row disappears; confirm the default
workspace shows no Rename/Delete; start a forecast and confirm both actions are refused.

## Risks and failure modes

- **Irreversible delete.** No backup gate exists (ADR-0008 unimplemented). The typed-name step is
  the only guard; the delete step must be clearly marked permanent in the UI.
- **Rename across volumes.** `std::fs::rename` fails if old and new paths are on different
  volumes; that is acceptable (workspace and its sibling share a volume). Report `io_error`.
- **Registry drift.** If the folder move succeeds but the registry save fails, the list points at
  the old path. Acceptable: the registry is advisory and the missing entry falls back to
  `exists=false`; do not roll back the move.
- **Partial delete.** `remove_dir_all` can fail midway; report `io_error` and leave the list row
  intact so the user can retry.

## Decisions deferred / open

- Trash or undo would need a new dependency or a staged app-data area — deferred.
- Bulk delete of several workspaces — not requested.
