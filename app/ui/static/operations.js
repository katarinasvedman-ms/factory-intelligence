const state = { incidents: [], selectedId: null };
const el = (id) => document.getElementById(id);

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

function technicalTextHtml(text, sourceCount) {
  return escapeHtml(text)
    .replace(/\[cite:(\d+)\]/gi, (_, value) => {
      const index = Number(value);
      return index >= 1 && index <= sourceCount
        ? `<a href="#proposal-source-${index}">[${index}]</a>`
        : `[source ${index}]`;
    })
    .replaceAll("\n", "<br>");
}

function statusClass(status) {
  if (status === "executed" || status === "resolved") return "success";
  if (status === "monitoring") return "monitoring";
  if (status === "rejected" || status === "failed") return "failure";
  return "preview";
}

function renderList() {
  el("incident-count").textContent = state.incidents.length;
  if (!state.incidents.length) {
    el("incident-list").className = "incident-list empty";
    el("incident-list").textContent = "No incidents yet.";
    return;
  }
  el("incident-list").className = "incident-list";
  el("incident-list").innerHTML = state.incidents.map((record) => `
    <button class="incident-row ${record.incident.incident_id === state.selectedId ? "selected" : ""}"
      data-incident-id="${escapeHtml(record.incident.incident_id)}">
      <span class="severity-dot ${escapeHtml(record.incident.alarm.severity)}"></span>
      <span>
        <strong>${escapeHtml(record.incident.machine_id)}</strong>
        <small>${escapeHtml(record.incident.vendor_profile)} · ${escapeHtml(display(record.incident.alarm.code))}</small>
      </span>
      <span class="badge ${statusClass(record.status)}">${escapeHtml(display(record.status))}</span>
    </button>
  `).join("");
  document.querySelectorAll("[data-incident-id]").forEach((button) => {
    button.addEventListener("click", () => {
      state.selectedId = button.dataset.incidentId;
      renderList();
      renderDetail();
    });
  });
}

function proposalHtml(record) {
  const proposal = record.proposal;
  if (!proposal) {
    const queued = record.incident.context.queued_for_sync === true;
    const monitoring = record.status === "monitoring";
    return `
      <article class="panel">
        <div class="section-kicker">FACTORY CORRELATION</div>
        <h2>${queued ? "Queued for synchronization" : monitoring ? "Monitoring downstream recovery" : "No action proposal"}</h2>
        <p class="muted">${
          queued
            ? "The machine handled this incident locally while factory operations were offline. Reconnect from Demo control to synchronize it."
            : monitoring
              ? "The governed action executed on the upstream machine. Monitor Packer 12 for restored product flow before resolving this downstream incident."
            : "This incident is informational, locally resolved, or correlated as a downstream effect."
        }</p>
      </article>`;
  }
  const citations = proposal.sources.length
    ? proposal.sources.map((source, index) => `
        <li id="proposal-source-${index + 1}">
          <span class="source-number">[${index + 1}]</span>
          <strong>${escapeHtml(source.title || "Factory source")}</strong>
          <span>${escapeHtml(source.source || "")}</span>
          ${source.excerpt ? `<p>${escapeHtml(source.excerpt)}</p>` : ""}
        </li>`).join("")
    : "<li>No external source was used for this governance test.</li>";
  const operatorSummary = proposal.operator_summary ||
    "Factory analysis produced a proposal. Review the governed action and supporting evidence before deciding.";
  const reduction = proposal.proposed_action.parameters.reduction_percent;
  const decision = record.guard_decision;
  const decisionHtml = decision ? `
    <div class="guard-result ${decision.permitted ? "permitted" : "blocked"}">
      <strong>${decision.permitted ? "Guard permitted and executed" : "Guard blocked the action"}</strong>
      <p>${escapeHtml(decision.reason)}</p>
      ${decision.outcome ? `<p>${escapeHtml(decision.outcome)}</p>` : ""}
    </div>` : "";
  const controls = record.status === "awaiting_approval" ? `
    <div class="approval-bar">
      <label>Operator <input id="operations-operator" value="Factory Operator" autocomplete="off"></label>
      <button data-decision="reject" class="danger-secondary">Reject</button>
      <button data-decision="approve">Approve through Guard</button>
    </div>` : "";
  return `
    <article class="panel proposal-panel">
      <div class="section-kicker">GROUNDED FACTORY PROPOSAL</div>
      <h2>${escapeHtml(proposal.root_cause_hypothesis)}</h2>
      <p class="large-copy operator-summary">${escapeHtml(operatorSummary)}</p>
      <div class="proposal-action">
        <span>Proposed governed action</span>
        <strong>${escapeHtml(display(proposal.proposed_action.action_id))}${reduction ? ` · ${reduction}%` : ""}</strong>
        <span class="badge preview">${escapeHtml(display(proposal.proposed_action.risk_level))} risk</span>
      </div>
      ${decisionHtml}
      ${controls}
      <details class="proposal-evidence" data-proposal-id="${escapeHtml(proposal.proposal_id)}">
        <summary>View technical analysis and ${proposal.sources.length} retrieved source${proposal.sources.length === 1 ? "" : "s"}</summary>
        <div class="proposal-evidence-body">
          <h3>Technical agent output</h3>
          <p class="technical-output">${technicalTextHtml(proposal.reasoning, proposal.sources.length)}</p>
          <h3>Retrieved sources</h3>
          <ol class="citation-list">${citations}</ol>
        </div>
      </details>
    </article>`;
}

function renderDetail() {
  const expandedProposalId = el("incident-detail")
    .querySelector(".proposal-evidence[open]")
    ?.dataset.proposalId;
  const record = state.incidents.find((item) => item.incident.incident_id === state.selectedId);
  if (!record) return;
  const incident = record.incident;
  el("incident-detail").innerHTML = `
    <article class="panel incident-summary">
      <div class="summary-title">
        <div>
          <div class="section-kicker">${escapeHtml(incident.line_id)} · ${escapeHtml(incident.vendor_profile)}</div>
          <h2>${escapeHtml(incident.machine_id)}</h2>
          <p>${escapeHtml(incident.alarm.raw_text)}</p>
        </div>
        <span class="badge ${statusClass(record.status)}">${escapeHtml(display(record.status))}</span>
      </div>
      <dl class="detail-list">
        <div><dt>Normalized alarm</dt><dd>${escapeHtml(display(incident.alarm.code))}</dd></div>
        <div><dt>Machine</dt><dd>${escapeHtml(incident.machine_model)} · ${escapeHtml(incident.firmware_version)}</dd></div>
        <div><dt>Manual revision</dt><dd>${escapeHtml(incident.manual_revision)}</dd></div>
        <div><dt>Connectivity</dt><dd>${escapeHtml(display(incident.connectivity))}</dd></div>
      </dl>
      <div class="local-assessment">
        <strong>Machine-local assessment</strong>
        <p>${escapeHtml(incident.local_assessment?.summary || "No local assessment recorded.")}</p>
      </div>
    </article>
    ${proposalHtml(record)}
    <article class="panel">
      <h2>Audit timeline</h2>
      <div class="timeline">
        ${record.audit.map((event) => `
          <article>
            <time>${escapeHtml(new Date(event.timestamp).toLocaleTimeString())}</time>
            <div><strong>${escapeHtml(display(event.event_type))}</strong><p>${escapeHtml(event.summary)}</p></div>
          </article>`).join("")}
      </div>
    </article>`;
  const proposalEvidence = el("incident-detail").querySelector(".proposal-evidence");
  if (proposalEvidence?.dataset.proposalId === expandedProposalId) {
    proposalEvidence.open = true;
  }
  document.querySelectorAll("[data-decision]").forEach((button) => {
    button.addEventListener("click", () => decide(button.dataset.decision));
  });
}

async function decide(decision) {
  const operatorId = document.getElementById("operations-operator")?.value || "";
  const response = await fetch(`/api/incidents/${state.selectedId}/${decision}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ operator_id: operatorId }),
  });
  const body = await response.json();
  if (!response.ok) {
    window.alert(body.detail || "Decision failed");
    return;
  }
  await refresh(false);
}

async function refresh(selectLatest = true) {
  const response = await fetch("/api/incidents");
  if (!response.ok) return;
  state.incidents = await response.json();
  if (selectLatest && !state.selectedId && state.incidents.length) {
    state.selectedId = state.incidents[0].incident.incident_id;
  }
  if (state.selectedId && !state.incidents.some((item) => item.incident.incident_id === state.selectedId)) {
    state.selectedId = state.incidents[0]?.incident.incident_id || null;
  }
  renderList();
  renderDetail();
}

refresh();
setInterval(() => refresh(false), 3000);
