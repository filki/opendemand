# ADR-0001: Desktop Application Stack — Tauri v2 with Static SvelteKit

**Status:** Accepted
**Date:** 2026-10-13

## Context

OpenDemand is a local-first demand planning desktop application for a single user working with a local workspace (SQLite database, source CSV copies, backups). It must install and run on Windows x64, macOS Apple Silicon, and Linux x64, keep all business data local, access the filesystem natively, and present a data-dense UI (tables, charts, editing) with predictable performance. The application also bundles a Python sidecar for forecasting (ADR-0003), which already consumes a large share of the installer budget.

## Decision Drivers

- Small installer and runtime footprint (the Python sidecar is the main size cost).
- Native filesystem access for workspaces, CSV copies, and backups.
- A typed, versioned boundary between UI and core logic.
- Productive UI development for data-dense screens (Svelte 5 runes, TanStack Query).
- Cross-platform desktop support from one codebase.

## Decision

Use **Tauri v2** as the desktop shell with a **static SvelteKit** frontend (adapter-static, no SSR), **Svelte 5 runes** for UI state, **TanStack Query** for server-state (command results, caching, invalidation), and **tauri-specta** to generate typed IPC bindings from Rust commands.

## Consequences

Positive:

- Small shell footprint; the system webview is reused instead of bundling a browser engine.
- Typed IPC: command signatures and result types are generated from Rust, so UI and core cannot drift silently.
- Static frontend: no Node.js at runtime and no dev server in production.
- One logic boundary: business logic, validation, and data access live in Rust only (ADR-0002).

Negative:

- Webview differences across platforms (WebView2, WKWebView, WebKitGTK) require cross-platform UI testing.
- Tauri and Rust add a learning curve and constrain some UI conveniences.
- A static build means all data must flow through typed commands.

## Alternatives Considered

- **Electron**: mature ecosystem, but runtime size and memory footprint conflict with the installer budget and the sidecar.
- **Flutter**: strong UI toolkit, but adds a third language ecosystem next to Rust and Python.
- **Pure web application**: cannot own local files and a local database as required.
- **PyQt/PySide**: unifies with the sidecar language, but a weaker ecosystem for data-dense web-style interfaces.
