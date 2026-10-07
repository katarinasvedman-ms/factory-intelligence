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

function render(record) {
  if (!record) return;
  const incident = record.incident;
  el("machine-name").textContent = incident.machine_id;
  el("machine-subtitle").textContent =
    `${incident.vendor_profile} · ${incident.machine_model} · Firmware ${incident.firmware_version}`;
  el("operating-state").textContent =
    record.status === "executed" ? "Controlled reduction active" :
    incident.alarm.severity === "critical" ? "Attention required" : "Running with advisory";
  el("connectivity").textContent = display(incident.connectivity);
  el("incident-status").textContent = display(record.status);
  el("alarm-title").textContent = `${incident.alarm.raw_code} · ${display(incident.alarm.severity)}`;
  el("alarm-text").textContent = incident.alarm.raw_text;
  el("vendor").textContent = incident.vendor_profile;
  el("machine-model").textContent = `${incident.machine_model} · Manual ${incident.manual_revision}`;
  el("normalized-code").textContent = display(incident.alarm.code);
  el("line-id").textContent = incident.line_id;
  el("local-assessment").textContent =
    incident.local_assessment?.summary || "The incident is awaiting a local assessment.";
  const action = el("candidate-action");
  if (incident.local_assessment?.candidate_action_id) {
    action.hidden = false;
    action.textContent = `Governed candidate: ${display(incident.local_assessment.candidate_action_id)}`;
  } else {
    action.hidden = true;
  }
  el("escalation-message").textContent =
    incident.connectivity === "offline"
      ? "Factory operations are offline. This incident is safely queued for synchronization."
      : incident.connectivity === "syncing"
        ? "Factory connectivity recovered. The queued incident is synchronizing now."
        : record.proposal
          ? "Factory correlation completed. The proposal is awaiting an operator decision in Governed Floor."
          : record.status === "locally_assessed"
            ? "Local assessment completed and inspection is recommended. The incident remains open until the alarm clears and recovery is verified."
            : "The incident has been escalated to factory operations.";
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
