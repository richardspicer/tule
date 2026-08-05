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
- Follow the monthly frontend dependency-review procedure in
  `docs/development.md` while automated version updates do not support the
  pinned pnpm version.
- Run formatting, static checks, tests, and the desktop build appropriate to the changed scope.
- Keep dependencies minimal and justify new privileged Tauri capabilities.

## Agent Operating Environment

- Primary implementation environment is Cursor. Project agents and the coding-loop
  rule live under `.cursor/` and are tracked with the repository.
- Lead, Builder (`implementer`), and Reviewer roles follow private project
  governance outside this repository. Repository agents must still obey the
  public boundaries in this file and `CONTRIBUTING.md`.
- Keep repository guidance model-agnostic. Do not hardcode vendor model
  identifiers in `AGENTS.md`, contributor docs, or coding-loop rules. Model
  selection is an owner/session preference; Cursor agent frontmatter may carry
  a local default that the owner can change.
- The Builder implements within an authorized write set and may commit or push
  only when that publication step is explicitly authorized for the task.
- The Builder must not create or edit pull requests, change pull-request
  metadata, post pull-request comments, or reply to or resolve review threads.
- Pull-request creation, merge, deployment, and release remain separate
  authorization gates from commit and push.
- App-level agent memories and chat recollections are non-authoritative. Prefer
  this file, `CONTRIBUTING.md`, architecture docs, and the authorized task brief.

## Public Change History

- Open authorized pull requests when publication is granted. Draft pull requests
  are permitted and may be used so automated review (including Copilot) can run
  before the PR is marked ready for human review or merge. Ready-for-review and
  draft are both valid publication states; choose draft when early automated
  review is useful. GitHub automation and CI review bots are not the primary
  implementation environment.
- Prefer a short product-facing pull-request description: what changed, why it
  was needed, user or developer impact, and the concise verification outcome.
  Mention every new dependency and any material permission, persistence,
  credential, or trust-boundary change. Do not add Test plan, Verification, or
  similar checklist sections; command lists; artifact paths; or private planning
  material.
- Treat a user request to address review comments as authorization to implement
  every unresolved actionable comment, reply with the result, and resolve each
  addressed thread. Leave ambiguous or conflicting comments open and report them.
- Keep pull-request titles and descriptions and commit subjects and bodies
  concise, product-facing, and understandable without repository knowledge.
- Never include implementation file names, directory paths, migration file names,
  machine-local paths, or private planning-artifact names in pull-request or
  commit prose.
- Describe changes through behavior, intent, user or developer impact,
  architectural or trust boundaries, and verification outcomes.
- Keep detailed implementation inventories in private planning and review records
  rather than public change history.

## Change Boundaries

- Do not add the Agent Harness, Hermes integration, ACP, plugins, publishing
  automation, or package-registry releases to the initial foundation.
- Keep credentials and machine-specific paths out of the repository.
- Treat commit, push, pull-request creation, merge, deployment, and release as
  distinct actions.
