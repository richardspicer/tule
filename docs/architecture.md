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
