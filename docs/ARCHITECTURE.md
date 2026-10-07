# Architecture

## Operator experiences

The application exposes three views over the same governed incident state:

```text
Machine HMI              Governed Floor              Demo control
local alarm              incident queue              deterministic replay
fast assessment          cross-machine correlation   environment diagnostics
connectivity             citations and approval      technical provider detail
machine timeline         Guard and audit
```

Provider selection, preflight, scenario replay and model details are presenter or
administrator concerns. They are not shown in the machine operator experience.

Two fictional vendor adapters normalize different alarm codes, tag names and units
into a shared `IncidentPackage`. Incidents and audit events are persisted in SQLite.
The factory view uses the existing providers for local assessment and Agentic
Retrieval, then validates every proposed action through the Guard.

## Offline operation and recovery

Factory connectivity is persisted in the SQLite `app_state` table. When the
presenter disconnects the factory cluster, machine-local inference remains
available. Incidents that require factory context are stored with
`queued_for_sync=true`, an `offline` connectivity state and an audit event.

Reconnect changes queued incidents to `syncing`, submits each incident to the
factory Agentic Retrieval path, stores the grounded proposal and citations, then
marks the incident `connected` and `awaiting_approval`. Synchronization selects only
records that still have `queued_for_sync=true` and no proposal, so repeated reconnect
operations are idempotent.

## Governed action boundary

The Guard is separate from model inference. It accepts only allowlisted action IDs,
checks parameters and incident state, requires an identified operator and records
both permitted and rejected decisions. The current demo permits `reduce_speed` only
between 5% and 30%. In the stopped-press scenario, this represents a simulated
recovery speed limit for a controlled restart after inspection, not an attempt to
slow a stationary machine. Model text is never treated as a command.

## Stable application contract

```text
Browser UI
   |
   v
Rust / Axum application
   |-- ScenarioService
   |-- RoutingPolicy
   |-- FastSlowCoordinator
   |-- InferenceService
   |-- GovernedFloorService
   |-- VendorSimulator
   |-- IncidentStore (SQLite)
   |-- GuardService
   |-- ProviderRegistry
   |
   v
IInferenceProvider
   |-- MockProvider
   `-- OpenAiCompatibleProvider
          |-- device configuration
          |-- edge configuration
          `-- cloud configuration
```

Scenario and coordination code uses only normalized domain models. Base URLs,
credentials, URL paths, authentication headers, and model identifiers remain provider
configuration.

## Fast and slow flow

1. The primary Robot 17 scenario always starts on the device fast path.
2. The application evaluates deterministic escalation rules.
3. Escalation freezes an immutable `EvidenceSnapshot`.
4. The planned slow path executes peer-machine and maintenance-history tools.
5. The Agentic Retrieval Agentic Layer runs the configured knowledge base.
6. The agent invokes the built-in indexed-source MCP server to retrieve local
   factory manuals and citations from the dedicated collection.
7. The application records the thread, run, tool steps, and citations.
8. When the edge result returns, the coordinator compares the current evidence
   version with the snapshot version.
9. Guidance is labeled `current` or `stale`. The contract also supports
   `revalidation_required` and `superseded`.

Model text is never translated directly into a machine command. A grounded proposal
is stored with its incident and shown in Governed Floor. After an identified operator
approves it, `GuardService` independently validates the allowlisted action, parameter
range and incident state. The current connector is simulated and does not command
physical equipment.

## Controlled action flow

1. A vendor alarm is normalized into the shared incident contract.
2. The machine-local fast agent produces a bounded assessment.
3. An incident requiring factory context is persisted and correlated with related
   machine events.
4. Agentic Retrieval produces a grounded proposal with source evidence.
5. Governed Floor shows the proposal to the factory operator.
6. A named operator approves or rejects the proposal.
7. `GuardService` checks the action ID, 5-30% limit, incident state and operator.
8. A permitted recovery limit reaches the simulated connector; an unsafe action is
   blocked. The connector records acceptance but commands no physical equipment.
9. Both permitted and rejected decisions are appended to the incident audit history.

Future physical connectors must preserve this contract and add environment-specific
authorization, interlocks, protocol validation, outcome verification, and recovery.

## Target routing

| Condition | Target |
|---|---|
| Routine single-machine event | Device |
| Shared factory context required | Edge cluster |
| Approved management/fleet analysis | Cloud |
| `local_only` classification | Device or edge only |

Explicit operator selection is allowed, but selecting cloud for `local_only` data is
rejected before a provider call.

Management reporting uses a separate governed data boundary:

1. The user selects a bounded time range and report mode.
2. The application filters persisted incidents and creates a minimized
   `cloud_allowed` JSON snapshot.
3. The UI displays the exact payload and excluded data categories without calling a
   model.
4. The server retains that immutable preview under a generated preview ID.
5. Explicit generation references the preview ID, ensuring the Expert receives the
   reviewed snapshot rather than client-supplied or newly selected data.
6. The application derives factual counts and outcomes; the Expert contributes only
   longer-term reliability recommendations and has no machine authority.

## Optional Fabric publication boundary

Fabric publication supplements rather than replaces the existing local management
brief. `IncidentStore` persists the mutable incident snapshot and inserts sanitized
audit-event projections into `fabric_outbox` within the same SQLite transaction.
The audit `event_id` is the outbox primary key, making repeated incident saves
idempotent.

```text
IncidentRecord + AuditEvent
        |
        v
allowlisted FabricIncidentEvent
        |
        v
SQLite fabric_outbox
        |
        v
background Event Hubs publisher
        |
        v
Fabric Eventstream -> Eventhouse
```

The publisher never participates in incident handling, operator approval, Guard
evaluation, machine recovery, or local brief generation. A Fabric failure records a
bounded error and retry schedule in the outbox while factory operations continue.
Publication is disabled unless `FABRIC_EXPORT_ENABLED=true`.

The fleet event projection includes factory, line, machine, incident, lifecycle
event, status, severity and explicitly approved detail fields. It excludes raw alarm
text, operator identity, model reasoning, manual content, credentials, endpoints and
connector details.

## Evidence and observability

Normalized responses expose:

- request and provider IDs;
- actual target and model;
- endpoint alias;
- latency and timestamp;
- success or structured error;
- fallback and mock status;
- token usage when returned.

Structured telemetry excludes prompt text, response content, tokens, credentials,
and authorization values.

## Environment-specific operations

Device model startup, Azure Local model deployment, local evaluation, and model
updates are not part of the portable scenario layer. They remain deployment or
operator workflows and must be validated against the target environment.

The slow path uses Agentic Retrieval in Foundry Local in `combined` mode. Agentic
Retrieval owns ingestion, chunking, embeddings, Milvus vector storage, PostgreSQL
metadata, retrieval, threads, runs, and citations. The default knowledge base calls
the built-in indexed-source MCP server for the factory-maintenance collection.
