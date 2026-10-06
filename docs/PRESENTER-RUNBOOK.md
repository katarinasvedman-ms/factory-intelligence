# Presenter runbook

This runbook covers the validated Governed Floor evaluation configuration in
`config\demo.aks-agentic.yaml`.

- Machine Fast uses live Qwen 2.5 0.5B through Foundry Local.
- Factory Slow uses Agentic Retrieval on Arc-connected AKS.
- Slow retrieves synthetic manuals through the authenticated manuals MCP source
  and uses the keyless `factory-slow-gpt5-mini` deployment.
- Expert uses the separate `factory-expert-gpt54` deployment.
- The Guard validates all machine-affecting proposals.
- The current connector is simulated and sends no command to physical equipment.

This is an Azure AKS evaluation deployment connected to Azure Arc. Do not describe
it as production Azure Local, disconnected Slow inference, or a production-ready
machine integration.

## Before the session

### Confirm the local model

```powershell
foundry service status
foundry service list
```

Expected:

- Foundry Local is available at `http://127.0.0.1:49195`.
- `qwen2.5-0.5b-instruct-openvino-gpu:2` is running.

If necessary:

```powershell
foundry model run qwen2.5-0.5b --device GPU --prompt "Reply with READY" --retain
```

### Start the application

Confirm `az account show` displays the intended subscription, then run:

```powershell
$env:FACTORY_AGENTIC_DOMAIN = "<agentic-domain>"
$env:FACTORY_AGENTIC_CLIENT_ID = "<entra-application-client-id>"
$env:FOUNDRY_BASE_URL = "https://<foundry-resource>.openai.azure.com/openai/v1"
$env:FOUNDRY_EXPERT_MODEL = "<expert-model-deployment>"
.\run-aks-agentic-demo.ps1
```

Open `http://127.0.0.1:8000/demo`.

Select **Reset demo** before beginning. Confirm the factory connection reads
**Connected**.

Reset only once at the beginning. The scenarios intentionally accumulate as events
from one factory shift so the final 24-hour management report can summarize locally
resolved, synchronized, rejected, executed and monitoring outcomes together.

## Presenter flow

### 1. Introduce Governed Floor

**Point at:** the title, architecture row and five guided scenarios.

**Say:**

> "A production line rarely comes from one vendor or one generation of equipment.
> When an upstream machine stops, downstream machines produce their own alarms, and
> operators must determine whether they are seeing several failures or one causal
> event. At the same time, no plant should place a probabilistic AI model inside its
> safety or real-time control loop."

Then introduce the response:

> "Governed Floor provides one governed flow for every machine and every vendor.
> Machine-local AI handles bounded work close to the equipment. Factory AI adds
> cross-machine context and grounded manuals. Operators and the Guard remain in
> control of every machine-affecting action. Approved outcomes can then become
> minimized fleet-level insight without centralizing unrestricted plant data."

The guided scenario cards are the presenter flow. The collapsed **Advanced provider
test** is a developer diagnostic console and is not part of the main demonstration.

### 2. Check the environments

**Click:** **Check environments**.

Expected:

| Environment | Expected state |
|---|---|
| `device` | Ready |
| `edge` | Ready |
| `cloud` | Ready |

**Say:**

> "Device is the live machine-local model. Edge is Agentic Retrieval on the
> Arc-connected AKS cluster. Cloud is the separate Expert deployment. Ready means
> the application reached the configured endpoint successfully."

Stop the main demo if Device or Edge is unavailable.

## Narrating live execution

The local model commonly takes several seconds. Agentic Retrieval can take longer
because it creates an agent run, invokes tools, retrieves documents and waits for
the Foundry model. Treat this as a visible part of the architecture rather than
silent waiting.

Keep the application terminal available on a second screen or in a narrow window.
The structured logs show events such as:

```text
request_started ... target=Device
provider_call_completed ... provider_id=device latency_ms=...
request_started ... target=Edge
provider_call_completed ... provider_id=edge latency_ms=...
```

These logs demonstrate that the application is progressing through real provider
calls. They intentionally do not contain credentials or full prompt and response
content.

During a wait:

1. Point at the changed button label, such as **Running local agent** or
   **Running factory flow**.
2. Explain which stage is active.
3. Optionally show the latest terminal event.
4. Return to the browser before navigation completes.

Do not promise an exact completion time or describe one observation as a latency
benchmark.

### 3. Routine local alarm

**Click:** **Routine local alarm — Run scenario**.

The button changes to **Running local agent** and then opens the Machine HMI.

**While it runs, say:**

> "The vendor alarm is first normalized into the common incident contract. The
> application is now calling the small Qwen model running locally on this machine.
> No AKS or cloud inference is required for this routine event."

**Optional terminal cue:** point at `request_started` with target `Device`, followed
by `provider_call_completed` for provider `device`.

**Point at:**

- `Mixer 08`;
- `Northstar Controls`;
- the raw vendor alarm and normalized alarm;
- Factory connection `Connected`;
- the local fast assessment;
- the machine timeline.

**Say:**

> "This is a bounded single-machine event. A Northstar-specific alarm was mapped
> into the common incident contract and assessed by the live local Qwen model. It
> did not require factory-wide context, so it was resolved locally."

Do not claim that the model diagnosis is independently verified. The result is
operator guidance.

Use **Demo control** in the navigation to return.

### 4. Cross-vendor cascade

**Click:** **Cross-vendor cascade — Run scenario**.

The browser opens Governed Floor after the local and factory stages complete.

**While it runs, say:**

> "This flow has two visible stages. First, the upstream machine performs its local
> fast assessment. The application then sends the normalized incident package and
> related machine context to the factory path. Agentic Retrieval creates a run,
> calls the authenticated manuals tool, retrieves evidence and asks the Slow model
> for a grounded proposal."

**Optional terminal cues:**

1. Show `request_started` and `provider_call_completed` for `Device`.
2. Show the following `request_started` event for `Edge`.
3. Explain that the Edge stage may remain active while tools and retrieval execute.
4. When available, show `provider_call_completed` for provider `edge`.

**Point at:** the incident queue.

The queue should contain:

- Northstar `Press 04`, the upstream drive incident;
- Contoso `Packer 12`, the downstream starvation symptom.

Select the Press 04 incident if it is not already selected.

**Point at:**

- the normalized alarm and machine-local assessment;
- the factory root-cause hypothesis;
- the concise operator summary;
- the proposed 15% recovery speed limit;
- the incident audit timeline.

**Say:**

> "The vendors use different codes, tag names and units, but both enter the same
> incident contract. The factory agent correlates the downstream symptom with the
> upstream stop and presents only the decision-relevant guidance. The complete
> model output and retrieved manuals remain available as evidence without
> overwhelming the operator."

Optionally expand **View technical analysis and sources**. Point out that citation
markers in the technical output link to the matching numbered source. Keep this
section collapsed during the primary flow unless the audience asks how the proposal
was grounded.

Enter the operator name and click **Approve through Guard**.

**Point at:** **Guard permitted and executed**.

**Say:**

> "Approval does not send model text to a machine. The Guard independently checks
> the action ID, the 5-to-30-percent limit, current incident state and operator
> identity. Because Press 04 is stopped, interpret this simulated request as
> installing a 15-percent-lower speed limit for a controlled restart after the
> required inspection—not as trying to slow a stationary machine. The current
> connector records acceptance only and sends no physical command."

### 5. Governance rejection

Return to **Demo control** without resetting, then run **Governance rejection**.

Governed Floor opens with a proposed 45% speed reduction.

**While it opens, say:**

> "This scenario deliberately stages an invalid proposal. It does not need a model
> call because the purpose is to demonstrate deterministic enforcement. The
> important step happens when the proposal reaches the Guard."

**Click:** **Approve through Guard**.

Expected result:

> `Requested speed reduction of 45% exceeds the governed 5–30% limit.`

**Say:**

> "The alarm text contains an untrusted instruction and the staged proposal exceeds
> policy. Even with operator approval, the Guard blocks it and records the reason.
> Model output and approval cannot bypass deterministic limits."

### 6. Network loss and recovery

Return to **Demo control** without resetting.

**Click:** **Disconnect factory**.

Confirm the status reads **Factory cluster offline**.

**Click:** **Network loss and recovery — Run while offline**.

The Machine HMI opens.

**While the local stage runs, say:**

> "Only the machine-local path is active. The factory connection is deliberately
> unavailable, so no factory-agent request is attempted. The incident and local
> assessment will be written to the persistent outbound queue."

**Optional terminal cue:** show the Device request completing without a subsequent
Edge request.

**Point at:**

- Factory connection `Offline`;
- the local Qwen assessment;
- the message that the incident is queued for synchronization;
- the persistent machine timeline.

**Say:**

> "The factory cluster is unavailable, but machine-local guidance continues. The
> incident is persisted locally rather than pretending that factory analysis ran."

Return to **Demo control** and click **Reconnect and sync**.

Wait for the status to report that one queued incident synchronized, then open
**Governed Floor**.

**While synchronization runs, say:**

> "Connectivity is restored. The application marks the queued incident as syncing
> and submits it to the factory path exactly once. Agentic Retrieval is now
> retrieving the relevant manuals and creating the grounded proposal that could not
> be produced while offline."

**Optional terminal cue:** show the new Edge request and its eventual
`provider_call_completed` event. Emphasize that there is no repeated Device call:
the persisted local assessment is reused.

**Point at:** the recovered proposal, citations and synchronization audit events.

**Say:**

> "After connectivity recovers, the queued incident is synchronized exactly once.
> Agentic Retrieval adds grounded factory context, and the incident moves to
> awaiting operator approval. Repeated reconnects do not duplicate the proposal."

### 7. Create the Incident Management Brief

Return to **Demo control** after the cross-vendor action has been approved and
Packer 12 has moved to **monitoring**.

**Click:** **Management brief — Build report**.

The browser opens **Incident Management Brief**. Keep the default scope:

- **Rolling past 24 hours**
- **Consolidated report**

The preview should now include the accumulated events from this demonstration shift,
not only the final cross-vendor incident.

Explain that a production user could instead select seven days, a custom time range,
or only the latest executed incident.

**Click:** **Preview approved data**.

**Point at:** the exact JSON payload and the explicitly excluded fields.

**Say:**

> "No model has been called yet. The application selected the incidents in the
> approved time window and constructed the exact payload that may leave the
> factory. Raw telemetry, vendor text, agent reasoning, retrieved manuals,
> operator identity, credentials and connector details are excluded."

Point out the server-held **Preview ID**, `cloud_allowed` classification and
**Machine authority: None**.

**Click:** **Approve payload and generate brief**.

**While it runs, say:**

> "Generation references the reviewed preview ID, so the Expert receives exactly
> the JSON we just inspected. The Expert contributes bounded longer-term reliability
> recommendations; it cannot select additional factory data, rewrite the operational
> record or control machinery."

**Optional terminal cue:** show the request using target `Cloud`.

**Point at:**

- the factual summary and counts derived by the application;
- the incident outcomes and governed action states;
- the bounded Expert reliability recommendations;
- the governance envelope showing `cloud_allowed` and no machine authority.

**Say:**

> "This is the operational-to-enterprise handoff. Machine and factory agents handled
> the immediate events. The Expert turns the explicitly reviewed reporting scope into
> a concise report for plant leadership and reliability teams, without participating
> in control. The demo currently uses incidents from one synthetic factory, but this
> cloud layer is intended to provide an approved view across the entire fleet of
> factories. Each factory retains its local operational authority while sharing only
> governed, minimized information for fleet-level analysis."

**Future-use narrative:**

> "In a production implementation, an operations manager could use this report for
> shift handover and impact review. A maintenance planner could create or prioritize
> inspection work, and a reliability engineer could use it to begin a root-cause
> review. At fleet level, corporate operations and reliability teams could compare
> recurring failure patterns, machine models, lines and maintenance outcomes across
> factories without centralizing unrestricted plant telemetry. Validated findings
> could later become versioned factory knowledge, but the generated report itself
> would never automatically become grounding. A reliability engineer would first
> confirm the root cause and repair outcome before publishing an approved maintenance
> record or knowledge article for future agent retrieval."

## Questions and answers

### Did the model decide to reduce speed by 15%?

> "The factory proposal contains a bounded action request. The operator decides
> whether to submit it, and the deterministic Guard independently checks it before
> the simulated connector can execute. In this scenario, the accepted request
> represents a recovery speed limit for a controlled restart after inspection. It is
> not an instruction to slow a press that is already stationary."

### Can a model call the connector directly?

> "No. Model text is untrusted advisory output. Only the Guard can pass a validated,
> allowlisted and operator-approved request to the connector."

### Is a physical machine changing speed?

> "No. The connector is simulated. No PLC, robot, MES, SCADA or safety-system
> command is sent."

### What remains active when the factory connection is down?

> "The machine-local model, normalized incident contract, local persistence and
> Machine HMI remain available. Factory correlation and grounded retrieval resume
> only after reconnect."

### Is observed latency a benchmark?

> "No. It is one observation in the current evaluation environment."

## Recovery

- **Scenario button appears inactive:** reload `/demo`. The button should change
  label immediately after selection.
- **Device unavailable:** confirm `foundry service status` and
  `foundry service list`.
- **Edge or Cloud unavailable:** restart with `.\run-aks-agentic-demo.ps1` to
  acquire fresh Microsoft Entra tokens.
- **Network-loss scenario is rejected:** select **Disconnect factory** first.
- **Reconnect fails:** confirm the Agentic endpoint, model bridge and manuals MCP
  are reachable, then select **Reconnect and sync** again. The queued incident is
  retained.
- **Guard blocks 15%:** confirm the incident is still `awaiting_approval` and enter
  a non-empty operator name.

## Closing statement

> "Governed Floor separates AI reasoning from operational authority. Local and
> factory agents interpret evidence, operators make decisions, and deterministic
> governance controls what may reach a machine. The same flow works across vendors
> without placing an agent in the safety or real-time control loop."
