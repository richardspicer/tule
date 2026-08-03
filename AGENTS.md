# Tule Repository Guidance

## Scope

These instructions apply to the entire repository.

## Product Boundary

- Tule owns its projects, workflows, permissions, provenance, artifacts, and history.
- Domain and application behavior belongs in `crates/tule-core` and must remain independent of Tauri.
- `apps/desktop` is the initial Tauri 2 host and React/TypeScript interface.
- The interface communicates with Rust through narrow typed commands. Do not expose unrestricted SQL, secrets, shell access, or filesystem access to frontend code.
- Model providers and future agent runtimes are replaceable adapters. Hermes and ACP are not initial dependencies.

## Development Contract

- Follow `CONTRIBUTING.md` and preserve the boundaries in
  `docs/architecture.md`.
- Respect `rust-toolchain.toml` and `.node-version`.
- Use the package-manager version declared in `package.json` through Corepack.
- Commit `Cargo.lock` and `pnpm-lock.yaml` for reproducible application builds.
- Run formatting, static checks, tests, and the desktop build appropriate to the changed scope.
- Keep dependencies minimal and justify new privileged Tauri capabilities.

## Public Change History

- Keep pull-request titles and descriptions and commit subjects and bodies concise, product-facing, and understandable without repository knowledge.
- Never include implementation file names, directory paths, migration file names, machine-local paths, or private planning-artifact names in pull-request or commit prose.
- Describe changes through behavior, intent, user or developer impact, architectural or trust boundaries, and verification outcomes.
- Keep detailed implementation inventories in private planning and review records rather than public change history.

## Change Boundaries

- Do not add the Agent Harness, Hermes integration, ACP, plugins, publishing automation, or package-registry releases to the initial foundation.
- Keep credentials and machine-specific paths out of the repository.
- Treat commit, push, pull-request creation, merge, deployment, and release as distinct actions.
