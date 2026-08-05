# Contributing to Tule

Tule is an early-stage desktop application. Contributions should keep the code
easy for both people and coding agents to inspect, change, and verify.

## Before You Begin

Follow the pinned toolchain and setup instructions in
[`docs/development.md`](docs/development.md). Keep each change focused on one
purpose and avoid adding infrastructure before the product needs it.

## Engineering Standards

- Prefer descriptive names, small focused modules, and direct control flow over
  clever abstractions.
- Comments should explain intent, invariants, tradeoffs, or security boundaries.
  Do not narrate code that is already clear. Keep comments current when behavior
  changes.
- Document public Rust domain APIs with Rustdoc. Document exported TypeScript
  contracts when their behavior, constraints, or failure modes are not obvious.
- Preserve strict TypeScript types. Avoid `any`, unchecked type assertions, and
  silently discarded errors.
- Return explicit Rust errors for expected failures. Reserve panics for
  unrecoverable startup failures and tests.
- Keep domain and application behavior in `crates/tule-core`, independent of
  Tauri. Keep React responsible for presentation and user interaction.
- Use narrow typed Tauri commands. Never expose generic SQL, secrets, shell
  execution, or unrestricted filesystem access to frontend code.
- Keep dependencies and native capabilities minimal. Explain every addition in
  the pull request.
- New dependencies must use a license permitted by dependency review. Expanding
  that policy requires explicit owner approval; do not bypass or disable the check.

## Tests and Verification

- Add tests for new domain behavior and regression tests for corrected defects.
- Test serialization and validation at native command boundaries when those
  contracts change.
- Prefer behavior-focused test names that state the expected outcome.
- Run the checks appropriate to the change. The standard local gates are:

  ```powershell
  corepack pnpm install --frozen-lockfile
  corepack pnpm check
  corepack pnpm build
  ```

  Native host or packaging changes also require an appropriate Tauri build.

## Git Conventions

- Name branches `<type>/<short-kebab-case-summary>` using an appropriate type
  such as `feature`, `fix`, `chore`, `docs`, `refactor`, or `test`.
- Do not use an author, coding tool, or agent identity such as `codex` or
  `agent` as the branch prefix.
- Keep commits focused. Use a concise imperative subject that states the change.

## Pull Requests

Keep titles factual and descriptions brief. Prefer one short product-facing
paragraph: what changed, why it was needed, user or developer impact, and the
concise verification outcome. Call out new dependencies, Tauri permissions,
persistence changes, credential handling, or changes to a trust boundary. Do
not add Test plan or Verification checklist sections, command lists, artifact
paths, or private planning material. Avoid promotional language. Never commit
credentials or machine-specific paths. Draft pull requests are permitted and may
be used so automated review can run before the pull request is marked ready.
