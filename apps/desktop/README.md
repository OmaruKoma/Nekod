# L1 — Desktop Application (`apps/desktop`)

**Status:** Player Desktop Client Implemented (P1–P5; Tauri 2 + React + TypeScript)

This directory hosts the player-facing Tauri 2 + React + TypeScript desktop application.

## Structure
- `src/` — React + TypeScript UI (`App.tsx`, `api/nekod.ts`, `styles.css`).
- `src-tauri/` — Tauri 2 Rust host exposing typed IPC commands backed exclusively by `crates/nekod-core`.

## Architectural Boundaries
- Presentation-only: communicates exclusively with `crates/nekod-core` (L2) via typed Tauri IPC commands.
- Contains zero OS process handles, memory scanning, or raw manifest parsing logic.
- Operates offline for local game discovery and trainer inspection with zero AI client or MCP server dependencies.

## Development Commands
```bash
npm install
npm run build
npm run tauri dev
```
