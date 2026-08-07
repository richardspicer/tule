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
event, lifecycle, conversation-context, and model-selection use cases. Core
prepares neutral request context (composed instructions, completed history,
current user text, optional Source framing, frozen model identifier, and optional
product Effort) and persists stable provider-profile and model identifier strings
on sessions and turns, plus nullable Effort provenance on each turn when that
control was available. It does not build chat/completions or Responses
request-body JSON, does not name provider wire keys such as `reasoning_effort` or
`service_tier`, and it does not own built-in provider-profile or default-model
constants. The desktop host persists non-secret session state in the shared
SQLite store, streams ordered channel events to the interface, and owns one xAI
subscription OAuth adapter (RFC 8628 device-code against `auth.x.ai`, catalog and
streamed chat/completions against `api.x.ai`). That adapter owns built-in profile
and upgrade-default model identifiers, a revisioned exact-id request-control
capability table, serialises the chat/completions wire body from core's neutral
context (including mapped Effort when capable), and performs network send.
Credentials stay in the OS credential store behind an opaque handle. The frontend
receives only typed connection status, allowlisted device pairing metadata
(verification URI and user code during connect), allowlisted model-catalog
metadata, selected-default state, allowlisted request-control capability facts,
and transcript data; it never receives authorization URLs beyond the allowlisted
verification URI, codes, tokens, account identifiers, raw catalog payloads,
provider instructions, tool definitions, or raw provider frames.

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
fallback or retry under another model.

Request controls follow a truthful-mapping rule: a product control (Effort or
Speed) is operable only when the active adapter can map the user's selection into
a documented request parameter for the session's frozen model and will actually
send that parameter. Unsupported controls are unavailable in the UI and rejected
if a client supplies a value anyway; they are never simulated. For the Phase 1
xAI chat/completions adapter, product Effort Low / Medium / High maps to wire
`reasoning_effort` `"low"` / `"medium"` / `"high"` only for exact model ids in
the adapter capability table (initially `grok-4.3` and `grok-4.5`, with documented
default `high`). Unknown or non-capable models, including `grok-3`, omit
`reasoning_effort`, keep Effort unavailable, and reject client-supplied Effort.
Speed has no truthful mapping on this path and remains unavailable (no
`service_tier` or invent-equivalent mapping). Effort may change per turn when
available; model freeze is unchanged. Ordinary send stays on streaming
chat/completions.

Tools, connectors, filesystem access,
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
Each hop resolves DNS once, rejects any blocked address in the result set, and
connects through a short-lived client pinned to those validated addresses while
still using the hostname for TLS SNI and certificate verification.
Responses also fail closed unless `Content-Type` is present and identifies
`text/*`, `application/json`, `application/xml`, `application/javascript`,
`application/ecmascript`, or `application/x-javascript`; missing, binary, and
unknown types are rejected before a Source, turn, event, or provider request is
created.
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

Session reopen through `get_agent_session` also returns the durable append-only
event sequence for that session in one round-trip, ordered by per-session
`sequence`. Each event DTO exposes only stable identity, optional turn linkage,
kind, and timestamp; there are no event payloads and no live event streaming on
the send channel. The React Activity panel is read-only inspectability beside
the turn transcript: collapsible rows keyed by event id with kind, timestamp,
and turn association when `turnId` is present. Hostile-response validation on
the platform layer rejects unknown event kinds and malformed event objects with
the same fail-closed posture as sessions and turns.

## Artifact Creation and Immutability

Artifacts extend the Agent boundary with durable product records created from
completed turns. Domain types, kind allowlisting, content hashing, provenance
freezing, reconstruction, and create/list/get use cases live in `tule-core`. The
desktop host owns the append-only `artifacts` / `artifact_versions` SQLite
schema, repository adapter, and typed commands `create_artifact_from_turn`,
`list_artifacts`, and `get_artifact`.

Create loads exact `agent_text` and turn provenance from durable storage by
`turn_id`. Client-supplied body or provenance is never trusted as source of
truth. Only turns in state `completed` with non-empty `agent_text` may create an
Artifact; other states and empty text are rejected without writing rows. Success
persists one Artifact and immutable version ordinal 1 atomically. Version
content is never updated or deleted in this slice; creating additional versions
is out of scope. Kind is allowlisted exactly as `conclusion`, `recommendation`,
`decision_record`, `requirements`, `implementation_plan`, `research_brief`, and
`critique`, defaulting to `conclusion`. Content SHA-256 uses the same
`hash_source_bytes` convention as Sources; reconstruction recomputes the digest
and fails closed on mismatch. Frozen provenance on the version copies
`source_session_id`, `source_turn_id`, `provider_profile_id`, `model_id`,
`prompt_version`, optional `project_id`, and `provider_request_id` from the
source turn at save time. Foreign keys enforce referenced session, turn,
provider-profile, and optional project rows on create; `provider_request_id` is
an opaque value copied from the turn and is not foreign-keyed (same as on
`agent_turns`). Session deletion behavior is not added.

List filter for the open Agent session is Artifacts with any version whose
`source_session_id` equals that session, union Artifacts whose `project_id`
equals the open session's project when present. Get returns the Artifact and
every version with full content and provenance. React receives only allowlisted
DTOs; the Agent workspace offers save on completed turns and a collapsible
Artifacts panel (not a top-level nav item) that can load and display a saved
version after reopen. Frontend validation fails closed on unknown kinds and
malformed artifact shapes. No new Tauri capabilities, webview CSP changes,
credentials, provider wire changes, or filesystem export are introduced.

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
