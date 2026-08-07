# TULE

TULE is an early-stage desktop workspace for structured AI-assisted thinking,
decisions, and implementation. This repository currently contains the native
desktop foundation: a Tauri host, a React and TypeScript interface, and a
Tauri-independent Rust core.

The first operational slice centers on an Agent workbench: local sessions,
optional Project context, and one xAI subscription OAuth adapter with
OS-backed credentials. Tools, filesystem or process access, plugins,
publication automation, and broader agent runtimes remain out of scope.

## Development

Windows is the supported development and release platform for the MVP. macOS
and Linux support are deferred until after the MVP. The supported Windows
toolchain and fresh-clone setup are documented in
[`docs/development.md`](docs/development.md). Once prerequisites are installed:

```powershell
corepack pnpm install --frozen-lockfile
corepack pnpm desktop:dev
```

Run the read-only environment check at any time:

```powershell
pwsh -File .\scripts\doctor.ps1
```

## Standards

Contribution, commenting, testing, and review expectations are documented in
[`CONTRIBUTING.md`](CONTRIBUTING.md). The current application boundaries are
described in [`docs/architecture.md`](docs/architecture.md).

Report suspected vulnerabilities privately by following
[`SECURITY.md`](SECURITY.md), not through a public issue.

## License

TULE is licensed under the [Apache License 2.0](LICENSE).
