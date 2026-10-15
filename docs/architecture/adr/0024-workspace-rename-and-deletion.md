# ADR-0024: Workspace Rename and Deletion

**Status:** Accepted
**Date:** 2026-10-15

## Context

A workspace is a user-chosen directory holding the SQLite database, retained sources, exports, backups, and job temp files (GLOSSARY.md; ADR-0002). The home flow could create, open, and forget list entries, but could not name a workspace and could not delete one: `workspaces_forget` removed only the recent-workspaces row. Users need a real rename and a real delete, and both must happen outside an open workspace so the workspace itself stays focused on forecasting.

Deletion is the most destructive operation in the product. The verified pre-migration backup gate (ADR-0008) is not implemented, so a deleted workspace cannot be restored by the application.

## Decision Drivers

- The existing name already is the folder name; a second name source would drift.
- Deletion must be deliberate and hard to trigger by accident.
- The default application-data workspace path is derived by the application; moving or deleting it would break that derivation.
- Commands must not touch anything outside the workspace directory.
- ADR-0009 never auto-deletes revisions or runs inside the database; workspace deletion is explicit, user-initiated, and workspace-wide.

## Decision

- **Name source of truth is the workspace folder name.** Rename moves the directory with `std::fs::rename` and re-points the recent-workspaces registry entry; no display-name field is stored.
- **Delete removes the entire workspace directory** (database, `staging/`, `sources/`, `recovery/`, `jobs/`, `exports/`, `backups/`) with `std::fs::remove_dir_all`. It is **permanent**; there is no OS trash and no new dependency. This is stated plainly in the confirmation UI.
- **Confirmation is two-step**, like GitHub's repository deletion: first an explicit "Are you sure?" step, then a text input that must exactly equal the current workspace name. The expected name is derived from the path in Rust, never taken from the caller; a mismatch fails with `confirm_name_mismatch`.
- **Guards:** both commands are refused while an import or forecast job is active (`workspace_busy` via `ensure_idle`), for the currently open workspace, for the default application-data workspace (`workspace_default`), and when no workspace database exists at the path (`workspace_not_found`). A rename target that already exists fails with `workspace_name_taken`; an invalid name fails with `invalid_workspace_name`.
- **Management lives on the workspace picker**, outside an open workspace. An open workspace keeps forecasting, import, weekly-update, and navigation controls only.
- A registry save failure after a successful move or delete does not roll back the filesystem operation; the registry is advisory (`recent.rs`) and the list degrades to `exists: false`.

## Consequences

Positive:

- The UI now matches the mental model of the domain: pick and manage workspaces outside, forecast inside.
- The typed-name step makes accidental deletion of the wrong workspace practically impossible.
- No schema change and no dependency; the two commands reuse the existing `AppError` code surface.

Negative:

- Deletion is irreversible; without the ADR-0008 gate there is no restore path in the application.
- `std::fs::rename` fails across volumes; workspaces always rename beside themselves, so this is acceptable but surfaces as a generic `io_error`.
- A partial `remove_dir_all` failure leaves a workspace on disk and reports `io_error`; the user retries.

## Alternatives Considered

- **A stored display name in the registry or `workspace_settings`**: rejected; two name sources drift, and the folder name remains the visible identity in the file manager anyway.
- **Moving the workspace to the OS trash**: rejected; platform trash needs a new dependency, which this feature does not justify.
- **Single-step confirmation (checkbox or dialog only)**: rejected for a permanent, whole-workspace delete; the typed name is the strongest cheap guard.
