# TULE Architecture

TULE is a local desktop application with a React interface, a narrow Tauri host,
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
            -> TULE core behavior
```

Responses return through the same typed boundary. Host response types may differ
from core types when that keeps serialization and interface concerns out of the
domain.

## Ownership

TULE owns its projects, workflows, permissions, provenance, artifacts, and
history. Model providers and future agent runtimes are replaceable adapters;
they do not own TULE's domain model. Hermes and ACP are not initial dependencies.

Persistence and provider integrations are introduced behind explicit Rust
interfaces. Their storage or transport details must not leak into domain
behavior or React components.

## Agent Sessions and Provider Boundary

The first Agent slice lives in `tule-core` as provider-neutral session, turn,
event, lifecycle, and model-selection use cases. The desktop host persists
non-secret session state in the shared SQLite store, streams ordered channel
events to the interface, and owns one xAI subscription OAuth adapter (RFC 8628
device-code against `auth.x.ai`, catalog and streamed chat/completions against
`api.x.ai`). Credentials stay in the OS credential store behind an opaque handle.
The frontend receives only typed connection status, allowlisted device pairing
metadata (verification URI and user code during connect), allowlisted model-catalog
metadata, selected-default state, and transcript data; it never receives
authorization URLs beyond the allowlisted verification URI, codes, tokens,
account identifiers, raw catalog payloads, provider instructions, tool
definitions, or raw provider frames.

After a successful connection, the native adapter fetches an account-aware model
catalog from `GET https://api.x.ai/v1/models` using the existing credentials,
disabled redirects, and truthful `User-Agent: tule-desktop/0.1.0`. Agent turns
stream through `POST https://api.x.ai/v1/chat/completions` with the same Bearer
and User-Agent; text messages only, no tools. Only allowlisted non-secret fields
are retained. The last validated catalog is cached in SQLite scoped to the
provider profile and credential generation—never the raw account identifier—with
retrieval time, ETag, and catalog-compatibility revision. A five-minute TTL marks
freshness; failures show last-known catalog as stale rather than substituting
hard-coded or public lists. An authenticated success with no usable models is a
bounded catalog failure and does not erase the last validated snapshot; the
refresh command still surfaces the bounded error rather than reporting stale
success. Catalog retrieval refreshes an expired access token under the shared
provider gate, and credential-generation invalidation fails closed on account
change or disconnect. A durable pre-transition quarantine hides catalog and
selection reads until invalidation commits or the original credential state is
restored, including across process restart when compensation and best-effort
scrubbing fail. Capability filtering excludes tool-only and Responses Lite
models without persisting those fields. Connection-status commands remain
separate from catalog and selected-default commands and events. On upgrade,
retired ChatGPT compatibility credential handles are cleared so the old adapter
cannot remain the active send path; historical sessions that recorded
`openai-chatgpt-compat` provenance remain readable.

A profile selected-model default persists independently of connection lifecycle
and of fixed profile display metadata. A new unsent Agent session may accept that
default or another catalog model; the first valid send freezes the chosen
provider-profile and model identifiers onto the durable session and every later
turn. Persisted sessions expose a non-editable model label and require a new
session to change models. Existing `grok-3` upgrade default and historical ChatGPT session provenance are
preserved separately. Allowlisted
model rejection surfaces as `model_unavailable`, refreshes or stales the catalog,
clears a rejected default, and requires a new-session choice without silent
fallback or retry under another model. Tools, connectors, filesystem access,
autonomous retries, and active-session model switching are out of scope for this
slice.

Turn-scoped local text Sources extend that Agent boundary. The native host owns
file-, folder-, and link-picker or URL intake, one-time bounded reading,
shallow enumeration, or HTTPS fetch, and ephemeral draft handles for the
main-window composer. React receives only an opaque draft handle and allowlisted
metadata—including the canonical requested URL string for link snapshots—and
never a path, file or directory content, listing, or fetched body. It gains no
generic dialog, filesystem, or network capability. On send, the host resolves
the handle, validates the snapshot in `tule-core`, and atomically persists the
Source with the pending turn and events before provider transmission. Folder
snapshots include only immediate-child regular UTF-8 text files (at most 32
members) under a single origin kind distinct from single-file attachments;
link snapshots use origin kind `remote_text_url` with a one-time native HTTPS
fetch at attach time (at most five redirect hops to other allowed HTTPS
targets; loopback, link-local, and private/reserved destinations fail closed).
Member count, aggregate byte count, canonical URL metadata, and deterministic
framed content round-trip through SQLite. Provider context includes the current
Source and Sources from completed prior turns in owning-turn order under prompt
version `tule-direct-agent-v2`, with explicit untrusted-data framing
subordinate to fixed and saved Project instructions. Routine IPC, transcripts,
events, and safe errors expose only Source identifier, origin kind, display
name, byte count, member count, hash, and—for link Sources—the canonical URL.
Failed, cancelled, and interrupted turns retain that metadata but do not enter
later context. The final serialized request still honors the existing 128 KiB
ceiling without truncation or silent omission.

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

Project instructions extend that same project-owned boundary. The core keeps
the instructions as exact plain text, permits empty content, and defines the
update use case and repository operation. The native adapter persists the text
through an append-only migration and bound SQLite parameters. The interface can
save instructions only through the named `update_project_instructions` command;
it receives the updated bounded project record and no storage details. This
does not add a frontend capability, rich-text interpretation, instruction
history, or a generic project-settings surface.

## Multi-Window Shell and Desktop Preferences

The desktop host owns a main workspace window and one singleton modeless
Settings window labeled `settings` with the native title **TULE — Settings**.
The main window keeps the native title bar with blank visible title text; the
application icon and accessibility identity remain. Both surfaces boot from the
same frontend package and branch on the native window label.

Settings opens or focuses through the typed `open_settings_window` command.
Closing Settings hides the existing window instead of exiting the application.
Optional category payloads (`providers` or `appearance`) travel on the
`settings-navigate` event so contextual deep links share one Settings
implementation. A normal reopen after Settings was hidden starts on Providers;
refocusing a visible Settings window preserves its selected category.
Appearance updates emit `appearance-changed`; authoritative non-secret
connection status emits `connection-status-changed`; allowlisted model catalog
and selected-default updates emit `provider-model-catalog-changed` and
`provider-model-selection-changed`. Event payloads contain only typed public
values—never credentials, authorization URLs or codes, tokens, account
identifiers, raw provider responses, SQLite paths, or internal errors.

Appearance preference persistence lives in the desktop SQLite owner behind an
explicit `appearance_preference` table and the typed
`get_appearance_preference` / `set_appearance_preference` commands. Invalid or
missing stored values resolve to `system`. A valid legacy webview `tule-theme`
value is imported into that native store once during upgrade and then retired;
the webview must not re-own durable preference storage. `appearance-changed`
emits the selected non-secret value even when a ready store rejects the write,
while the command still returns the bounded persistence error so open windows
stay visually synchronized. Storage failure must not prevent the shell from
opening and must not expose storage details across IPC.

Tauri capabilities grant event listen/unlisten only to the named `main` and
`settings` windows. Accepted main-window close prevents per-window destruction
and exits the application; Settings close hides the singleton instead. The host
does not expose generic window creation, filesystem, shell, SQL, or credential
access to either webview.

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
