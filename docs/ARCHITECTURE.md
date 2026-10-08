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
The machine process performs local assessment and exchanges durable advisory
request/response messages with the AKS worker through MQTT. The worker owns Agentic
Retrieval. Both operator views read the same machine-side incident state, and every
proposed action is validated through the local Guard.

## Factory messaging boundary

```text
Machine Rust application
  -> SQLite advisory_outbox
  -> MQTT QoS 1 request
  -> Mosquitto in the factory Kubernetes cluster
  -> factory-advisory-worker
  -> Agentic Retrieval and manuals MCP
  -> MQTT QoS 1 response
  -> SQLite advisory_inbox
  -> Machine HMI / Factory Operations approval
  -> local Edge Guard and connector
```

Request and response message IDs are stable. The machine retains a request until a
correlated response is applied, ignores duplicate responses, and checks the
`evidence_version` before changing incident state. The worker persists job state and
replays the stored response for an already completed request. MQTT transport is
therefore at-least-once while advisory application and machine execution remain
idempotent.

`ADVISORY_TRANSPORT=direct` preserves the earlier single-process development mode.
`ADVISORY_TRANSPORT=mqtt` is the separated demo mode: the machine process does not
invoke Agentic Retrieval for routine advisories.

## Offline operation and recovery

Factory connectivity is persisted in the SQLite `app_state` table. When the
presenter disconnects the factory cluster, machine-local inference remains
available. Incidents that require factory context are stored with
`queued_for_sync=true`, an `offline` connectivity state and an audit event.

Reconnect changes queued incidents to `syncing`. In MQTT mode it transactionally
enqueues an advisory request; the background publisher sends it when the broker is
available. The correlated response stores the grounded proposal and citations, then
marks the incident `connected` and `awaiting_approval`. Synchronization selects only
records that still have `queued_for_sync=true` and no proposal, so repeated reconnect
operations and message redelivery are idempotent.

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
   |-- MqttAdvisoryMachine
   |-- GuardService
   |-- ProviderRegistry
   |
   +--> Device provider
   +--> MQTT broker --> factory-advisory-worker --> Agentic Retrieval provider
   `--> Cloud provider
```

Scenario and coordination code uses only normalized domain models. Base URLs,
credentials, URL paths, authentication headers, and model identifiers remain provider
configuration.

## Fast and slow flow

1. A machine scenario starts on the device fast path.
2. The application evaluates deterministic escalation rules and persists the
   normalized incident.
3. Escalation freezes an immutable `EvidenceSnapshot` and creates a durable advisory
   request.
4. The machine publishes that request to the factory MQTT topic.
5. The AKS worker claims the request and runs the configured Agentic Retrieval
   knowledge base.
6. The agent invokes the authenticated manuals MCP server to retrieve local
   factory manuals and citations from the dedicated collection.
7. The worker publishes a summarized advisory and bounded action proposal to the
   machine-specific response topic.
8. The machine compares the response evidence version with the current incident,
   stores citations and displays the proposal on both operator views.
9. Approval and Guard execution remain local to the machine application.

Model text is never translated directly into a machine command. A grounded proposal
is stored with its incident and shown in Governed Floor. After an identified operator
approves it, `GuardService` independently validates the allowlisted action, parameter
range and incident state. The current connector is simulated and does not command
physical equipment.

## Controlled action flow

1. A vendor alarm is normalized into the shared incident contract.
2. The machine-local fast agent produces bounded observations without claiming
   grounded vendor diagnosis.
3. An incident requiring guidance is persisted with a durable MQTT request.
4. The AKS worker uses Agentic Retrieval to produce a grounded proposal with source
   evidence and returns it over MQTT.
5. The Machine HMI and Factory Operations show the same proposal.
6. A named operator approves or rejects from either view; the first decision wins.
7. `GuardService` checks the action ID, 5-30% limit, incident state and operator.
8. A permitted recovery limit reaches the simulated connector; an unsafe action is
   blocked. The connector records acceptance but commands no physical equipment.
9. Both permitted and rejected decisions are appended to the incident audit history.

Future physical connectors must preserve this contract and add environment-specific
authorization, interlocks, protocol validation, outcome verification, and recovery.

## Target routing

| Condition | Target |
|---|---|
| Immediate machine observation | Device |
| Grounded connected-machine advisory | AKS worker through MQTT |
| Shared factory context required | AKS worker through MQTT |
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
