# Tule Architecture

Tule is a local desktop application with a React interface, a narrow Tauri host,
and a framework-independent Rust core. This document describes the current
boundaries; it does not commit the project to unimplemented features.

## Repository Structure

- `crates/tule-core` owns domain and application behavior. It must not depend on
  Tauri or a particular interface.
- `apps/desktop/src-tauri` owns the desktop process, Tauri configuration, and
  narrowly scoped commands that adapt the host to the core.
- `apps/desktop/src` owns React presentation, interaction, and local view state.
  It should not become the authoritative home of domain rules.

The normal call path is:

```text
React interface
    -> named, typed Tauri command
        -> host-side validation and adaptation
            -> Tule core behavior
```

Responses return through the same typed boundary. Host response types may differ
from core types when that keeps serialization and interface concerns out of the
domain.

## Ownership

Tule owns its projects, workflows, permissions, provenance, artifacts, and
history. Model providers and future agent runtimes are replaceable adapters;
they do not own Tule's domain model. Hermes and ACP are not initial dependencies.

Persistence and provider integrations should be introduced behind explicit Rust
interfaces when needed. Their storage or transport details must not leak into
domain behavior or React components.

## Project Persistence

The project model and its application operations live in `tule-core`. The core
generates opaque UUID version 7 project identifiers, normalizes and validates
display names, records creation time, and defines the repository interface used
to create, list, and open projects. Project names are labels rather than keys,
so duplicate display names are valid.

The desktop host implements that repository with a single serialized SQLite
connection. It resolves a fixed database filename beneath Tauri's application
local data directory, enables foreign-key enforcement, and applies validated,
embedded, append-only migrations before project commands can use the store.
SQLite paths, connections, migrations, statements, and errors remain native
implementation details.

If path resolution, directory creation, database opening, or migration fails,
the desktop shell still starts with project storage marked unavailable. The
interface can only call the named `create_project`, `list_projects`, and
`open_project` commands. Those commands run blocking persistence work away from
the main thread and return minimal project records or one of four allowlisted
error codes; they never expose the database path, raw SQL, or internal errors.
No frontend capability is required for project persistence.

## Trust Boundary

The webview is not a privileged execution environment. Native commands must
validate untrusted input and return only the data required by the interface.
Tauri capabilities should grant the least privilege needed for a specific user
operation.

Do not expose unrestricted filesystem access, shell execution, SQL execution,
credentials, or provider secrets to frontend code. Sensitive values should
remain in the native layer and must not appear in logs or error messages.

## Change Rule

New behavior belongs in the lowest layer that can own it without depending on a
higher layer. If a change requires a new dependency, native capability, or trust
boundary, document the reason and validation in its pull request.
