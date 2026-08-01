# Windows development environment

Tule's Windows desktop environment is pinned so the same commit can be built on the laptop and a separate workstation.
The guided setup supports x64 and ARM64 Windows workstations.
Windows is the supported development and release platform for the MVP. macOS and Linux support are deferred until after the MVP.

| Tool | Required version or feature |
| --- | --- |
| Rust | 1.97.1 with the native Windows MSVC target, `rustfmt`, and `clippy` |
| Project Node.js | Exactly 24.18.1, installed on the host or provisioned in the repository |
| Bootstrap Node.js | Node.js 22.13 or newer within 22.x, or Node.js 24.x, with Corepack |
| pnpm | Exactly 11.4.0 through Corepack and the committed `packageManager` field |
| Visual Studio | Desktop development with C++, the compiler for the host architecture, and a Windows SDK |
| WebView2 | Registered Microsoft Evergreen WebView2 Runtime |
| Git | Git for Windows available on `PATH` |

The source-of-truth pins are `rust-toolchain.toml`, `.node-version`, and the root `package.json`. Lockfiles are committed for application builds.

Node.js 24.18.1 may be installed system-wide. Alternatively, Node.js 22.13 or newer within the 22.x release line, or Node.js 24.x, can bootstrap Corepack and pnpm 11. During the first online locked install, pnpm follows the committed `devEngines.runtime` policy and provisions Node.js 24.18.1 under `node_modules\.bin`. Subsequent repository commands use that exact managed runtime. If the managed runtime is absent, the doctor requires the host Node.js runtime to be exactly 24.18.1. Node.js 25 and newer do not bundle Corepack and are outside this bootstrap path.

Repository scripts invoke `corepack pnpm`. Corepack selects pnpm 11.4.0 and verifies it against the integrity-pinned `packageManager` field. The doctor deliberately does not execute an arbitrary direct `pnpm` command found on `PATH`; it reports that command as a warning because it does not control the package-manager version used by the repository scripts.

## Check the machine

From the repository root, run:

```powershell
pwsh -NoProfile -File .\scripts\doctor.ps1
```

Windows PowerShell 5.1 can also run the script. The process-only execution-policy option below does not change the machine or user policy:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\doctor.ps1
```

The doctor is offline and read-only. It reads version commands, Visual Studio installation metadata, the Windows SDK, and the documented WebView2 registry locations. It disables rustup automatic installation, Corepack network access, and Corepack automatic pinning while it checks the toolchains. It does not install, update, enable, download, build, or edit anything.

Exit codes are:

- `0`: all required checks passed; warnings may remain.
- `1`: one or more machine prerequisites are missing or mismatched.
- `2`: the host is unsupported, a repository pin is missing or malformed, or the doctor itself could not complete.

Follow only the remediation for failed checks, restart the terminal when a tool or `PATH` changed, and rerun the doctor.

## Prepare a new workstation

1. Install [Git for Windows](https://git-scm.com/download/win). Install Node.js 24.18.1 from the [Node.js downloads](https://nodejs.org/en/download), or use Node.js 22.13+ within 22.x or Node.js 24.x only to bootstrap the managed runtime during the first online install.
2. Install [rustup](https://rustup.rs/) with the native MSVC host. The committed toolchain file selects Rust 1.97.1. If the doctor reports it missing, use its exact `rustup` remediation command.
3. Install [Visual Studio Community or Build Tools](https://visualstudio.microsoft.com/downloads/). In Visual Studio Installer, select **Desktop development with C++**, the MSVC compiler for the workstation architecture, and a current Windows SDK.
4. Install or repair the [Microsoft Evergreen WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/consumer/) if the doctor cannot find a registered runtime.
5. From the repository root, hydrate the pinned package manager, locked dependencies, and managed Node.js runtime while online:

   ```powershell
   corepack install
   corepack pnpm@11.4.0 install --frozen-lockfile
   ```

6. Run the doctor until it exits with code `0`.
7. Run repository checks through the committed scripts, which invoke the exact Corepack-managed pnpm version.

The direct pnpm shim does not need to be enabled. Provider credentials are not part of the repository; configure them separately through the operating-system credential store when provider support is introduced.

## Validate the foundation

Run repository checks only from a trusted checkout. The checks execute project
and dependency code, may download locked dependencies when the local caches are
empty, and create normal build, test, and cache artifacts.

From the repository root, run the aggregate check and production web build:

```powershell
corepack pnpm check
corepack pnpm build
```

The aggregate check runs frontend formatting, type-aware linting, type checks,
and tests plus Rust formatting, Clippy with warnings denied, and all Rust tests
against the locked dependency graph. GitHub CI runs the same check on Windows
and also builds the configured native installer.
The equivalent individual Rust commands are:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

To validate a release-mode desktop executable without creating an installer:

```powershell
corepack pnpm --filter @tule/desktop tauri build --no-bundle -- --locked
```

## Detection notes

If rustup has just been installed but the current process inherited an older `PATH`, the doctor also checks the standard per-user `.cargo\bin` directory. It uses the discovered tools for validation and warns that the terminal must be restarted; it does not report Rust as absent.

The Visual Studio check accepts Build Tools and full Visual Studio editions. It uses `vswhere.exe` when available and falls back to compiler, linker, MSBuild, and Windows SDK files beneath standard Windows installation roots. No installation path is printed or committed.

The WebView2 check uses Microsoft's Evergreen Runtime client identifier, `{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`, in both machine and current-user registry locations. A valid registration and a matching runtime executable are both required.
