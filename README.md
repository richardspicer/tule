# Tule

Tule is an early-stage desktop workspace for structured AI-assisted thinking,
decisions, and implementation. This repository currently contains the native
desktop foundation: a Tauri host, a React and TypeScript interface, and a
Tauri-independent Rust core.

The product surface is still being shaped. The initial foundation deliberately
does not include provider integrations, an agent runtime, plugins, or publishing
automation.

## Development

The supported Windows toolchain and fresh-clone setup are documented in
[`docs/development.md`](docs/development.md). Once prerequisites are installed:

```powershell
corepack pnpm install --frozen-lockfile
corepack pnpm desktop:dev
```

Run the read-only environment check at any time:

```powershell
pwsh -File .\scripts\doctor.ps1
```
