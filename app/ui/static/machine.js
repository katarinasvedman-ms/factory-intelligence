const el = (id) => document.getElementById(id);
let activeIncidentId = null;

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function display(value) {
  return String(value ?? "").replaceAll("_", " ");
}

function advisoryHtml(record) {
  const proposal = record.proposal;
  const triage = record.incident.context.local_triage;
  const correlatedDownstream = record.audit.some(
    (event) => event.event_type === "correlated_as_downstream_effect",
  );
  if (!proposal) {
    if (correlatedDownstream) {
      const monitoring = record.status === "monitoring";
      return `
        <div class="section-kicker">FACTORY CORRELATION</div>
        <h2>${monitoring ? "Monitoring downstream recovery" : "Correlated downstream effect"}</h2>
        <div class="advisory-waiting">
          <strong>${monitoring ? "Upstream action executed" : "No separate machine advisory required"}</strong>
          <p class="muted">${
            monitoring
              ? `Monitor ${escapeHtml(record.incident.machine_id)} for restored product flow before resolving this downstream incident.`
              : "Factory operations linked this alarm to an upstream machine incident. Review and govern the proposed action on the upstream machine."
          }</p>
        </div>`;
    }
    const queued = triage?.advisory_status === "queued";
    const failed = triage?.advisory_status === "failed";
    return `
      <div class="section-kicker">FACTORY ADVISORY AGENT</div>
      <h2>${queued ? "Advisory queued" : failed ? "Advisory unavailable" : "Awaiting grounded guidance"}</h2>
      <div class="advisory-waiting">
        <strong>${queued ? "Factory connection unavailable" : failed ? "The request did not complete" : "Analysis in progress"}</strong>
        <p class="muted">${
          queued
            ? "The incident is stored locally and will be submitted when connectivity recovers."
            : failed
              ? "No diagnosis or machine action was produced. Review the audit timeline before retrying."
              : "The advisory agent is retrieving applicable manuals and evaluating the normalized evidence. The machine remains under local observation."
        }</p>
      </div>`;
  }

  const grounded = proposal.sources.length > 0;
  const reduction = proposal.proposed_action.parameters.reduction_percent;
  const citations = grounded
    ? proposal.sources.map((source, index) => `
        <li>
          <span class="source-number">[${index + 1}]</span>
          <strong>${escapeHtml(source.title || "Factory source")}</strong>
          <span>${escapeHtml(source.source || "")}</span>
          ${source.excerpt ? `<p>${escapeHtml(source.excerpt)}</p>` : ""}
        </li>`).join("")
    : "<li>No grounded source was returned. Approval is unavailable.</li>";
  const decision = record.guard_decision;
  const decisionHtml = decision ? `
    <div class="guard-result ${decision.permitted ? "permitted" : "blocked"}">
      <strong>${decision.permitted ? "Edge Guard permitted and executed" : "Action not executed"}</strong>
      <p>${escapeHtml(decision.reason)}</p>
      ${decision.outcome ? `<p>${escapeHtml(decision.outcome)}</p>` : ""}
    </div>` : "";
  const controls = record.status === "awaiting_approval" && grounded ? `
    <div class="approval-bar">
      <label>Machine operator <input id="machine-operator" value="Machine Operator" autocomplete="off"></label>
      <button data-machine-decision="reject" class="danger-secondary">Reject</button>
      <button data-machine-decision="approve">Approve through Edge Guard</button>
    </div>` : "";
  return `
    <div class="section-kicker">${grounded ? "GROUNDED FACTORY ADVISORY" : "UNGROUNDED FACTORY RESPONSE"}</div>
    <h2>${escapeHtml(proposal.root_cause_hypothesis)}</h2>
    <p class="large-copy operator-summary">${escapeHtml(proposal.operator_summary)}</p>
    <div class="proposal-action">
      <span>Proposed machine action</span>
      <strong>${escapeHtml(display(proposal.proposed_action.action_id))}${reduction ? ` · ${reduction}%` : ""}</strong>
      <span class="badge preview">${escapeHtml(display(proposal.proposed_action.risk_level))} risk</span>
    </div>
    ${decisionHtml}
    ${controls}
    <details class="proposal-evidence" data-proposal-id="${escapeHtml(proposal.proposal_id)}">
      <summary>View technical response and ${proposal.sources.length} retrieved source${proposal.sources.length === 1 ? "" : "s"}</summary>
      <div class="proposal-evidence-body">
        <h3>Advisory-agent response</h3>
        <p class="technical-output">${escapeHtml(proposal.reasoning).replaceAll("\n", "<br>")}</p>
        <h3>Retrieved sources</h3>
        <ol class="citation-list">${citations}</ol>
      </div>
    </details>`;
}

function render(record) {
  if (!record) return;
  const expandedProposalId = el("machine-advisory")
    .querySelector(".proposal-evidence[open]")
    ?.dataset.proposalId;
  activeIncidentId = record.incident.incident_id;
  const incident = record.incident;
  el("machine-name").textContent = incident.machine_id;
  el("machine-subtitle").textContent =
    `${incident.vendor_profile} · ${incident.machine_model} · Firmware ${incident.firmware_version}`;
  el("operating-state").textContent =
    record.status === "executed" ? "Controlled reduction active" :
    incident.alarm.severity === "critical" ? "Attention required" : "Running under observation";
  el("connectivity").textContent = display(incident.connectivity);
  el("incident-status").textContent = display(record.status);
  el("alarm-title").textContent = `${incident.alarm.raw_code} · ${display(incident.alarm.severity)}`;
  el("alarm-text").textContent = incident.alarm.raw_text;
  el("vendor").textContent = incident.vendor_profile;
  el("machine-model").textContent = `${incident.machine_model} · Manual ${incident.manual_revision}`;
  el("normalized-code").textContent = display(incident.alarm.code);
  el("line-id").textContent = incident.line_id;
  el("local-assessment").textContent =
    incident.local_assessment?.summary ||
    (record.audit.some((event) => event.event_type === "correlated_as_downstream_effect")
      ? "No separate local assessment was required. Factory operations correlated this alarm as a downstream effect."
      : "The incident is awaiting a local observation.");

  const action = el("candidate-action");
  if (record.proposal) {
    action.hidden = false;
    action.textContent = `Factory proposal: ${display(record.proposal.proposed_action.action_id)}`;
  } else {
    action.hidden = true;
  }

  el("escalation-message").textContent =
    incident.connectivity === "offline"
      ? "Factory guidance is unavailable. The incident is queued for synchronization."
      : incident.connectivity === "syncing"
        ? "Connectivity recovered. The factory advisory request is synchronizing."
        : record.status === "correlating"
          ? "Local observation is complete. The machine is awaiting grounded guidance from the factory advisory agent."
          : record.status === "monitoring"
            ? "The upstream governed action executed. Monitor downstream product flow before resolving this incident."
          : record.status === "awaiting_approval"
            ? "Grounded guidance is available. Approve here or from Factory Operations; both views use the same decision."
            : record.status === "executed"
              ? "The approved action passed the Edge Guard and was executed once."
              : record.status === "rejected"
                ? "The proposed action was rejected and was not executed."
                : "Review the factory advisory state below.";

  el("machine-advisory").innerHTML = advisoryHtml(record);
  const proposalEvidence = el("machine-advisory").querySelector(".proposal-evidence");
  if (proposalEvidence && proposalEvidence.dataset.proposalId === expandedProposalId) {
    proposalEvidence.open = true;
  }
  document.querySelectorAll("[data-machine-decision]").forEach((button) => {
    button.addEventListener("click", () => decide(button.dataset.machineDecision));
  });

  el("machine-timeline").className = "timeline";
  el("machine-timeline").innerHTML = record.audit.map((event) => `
    <article>
      <time>${escapeHtml(new Date(event.timestamp).toLocaleTimeString())}</time>
      <div>
        <strong>${escapeHtml(display(event.event_type))}</strong>
        <p>${escapeHtml(event.summary)}</p>
      </div>
    </article>
  `).join("");
}

async function decide(decision) {
  const operatorId = document.getElementById("machine-operator")?.value || "";
  const response = await fetch(`/api/incidents/${activeIncidentId}/${decision}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      operator_id: operatorId,
      decision_source: "machine_hmi",
    }),
  });
  const body = await response.json();
  if (!response.ok) {
    window.alert(body.detail || "Decision failed");
    return;
  }
  render(body);
}

async function refresh() {
  try {
    const response = await fetch("/api/incidents");
    if (!response.ok) throw new Error("Could not load machine incidents");
    const incidents = await response.json();
    render(incidents[0]);
  } catch (error) {
    el("local-assessment").textContent = error.message;
  }
}

refresh();
setInterval(refresh, 3000);
