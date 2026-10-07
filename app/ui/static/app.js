const state = {
  scenarios: [],
  providers: [],
  currentRun: null,
  currentAction: null,
  factoryConnected: null,
};

const el = (id) => document.getElementById(id);

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function scenarioChanged() {
  const scenario = state.scenarios.find((item) => item.id === el("scenario").value);
  if (!scenario) return;
  el("prompt").value = scenario.default_prompt;
  el("scenario-description").textContent = scenario.description;
  const enabledTargets = new Set(
    state.providers.filter((provider) => provider.enabled).map((provider) => provider.target_type)
  );
  for (const option of el("target").options) {
    if (option.value !== "auto") option.disabled = !enabledTargets.has(option.value);
  }
  el("target").value =
    scenario.recommended_target === "auto" && !enabledTargets.has("edge")
      ? "device"
      : scenario.recommended_target;
  resetAction();
}

function responseHtml(response) {
  if (!response) return '<span class="muted">Not used for this run.</span>';
  const status = response.success ? "success" : "failure";
  const content = response.success ? response.content : response.error?.message;
  return `
    <div class="response-header">
      <span class="badge ${response.mock ? "mock" : status}">${response.mock ? "SIMULATED" : status.toUpperCase()}</span>
      <span>${escapeHtml(response.target_type)} · ${escapeHtml(response.model_id)}</span>
    </div>
    <p>${escapeHtml(content)}</p>
  `;
}

function evidenceHtml(run) {
  const responses = [run.fast_response, run.slow_response].filter(Boolean);
  const rows = responses.map((response) => `
    <tr>
      <td>${escapeHtml(response.request_id)}</td>
      <td>${escapeHtml(response.target_type)}</td>
      <td>${escapeHtml(response.provider_id)}</td>
      <td>${escapeHtml(response.model_id)}</td>
      <td>${escapeHtml(response.endpoint_alias)}</td>
      <td>${response.latency_ms} ms</td>
      <td>${response.success ? "Success" : "Failure"}</td>
    </tr>
  `).join("");
  return `
    <div class="run-summary">
      <strong>${escapeHtml(run.mode)}</strong>
      <span>Routing rule: ${escapeHtml(run.routing.rule_id)}</span>
      <span>${escapeHtml(run.routing.reason)}</span>
      ${run.escalation ? `<span>Escalation: ${escapeHtml(run.escalation.rule_id)} — ${escapeHtml(run.escalation.reason)}</span>` : ""}
      ${run.guidance_status ? `<span>Guidance: ${escapeHtml(run.guidance_status)}</span>` : ""}
    </div>
    <div class="table-wrap">
      <table>
        <thead><tr><th>Request ID</th><th>Target</th><th>Provider</th><th>Model</th><th>Endpoint alias</th><th>Latency</th><th>Outcome</th></tr></thead>
        <tbody>${rows}</tbody>
      </table>
    </div>
  `;
}

function agenticEvidenceHtml(run) {
  const evidence = run.slow_response?.agentic_evidence;
  if (!evidence) {
    return '<span class="muted">The slow provider did not return grounded retrieval evidence.</span>';
  }
  const steps = evidence.steps.map((step) => `
    <li>
      <strong>${escapeHtml(step.step_type)}</strong>
      ${step.tool_name ? ` · ${escapeHtml(step.tool_name)}` : ""}
    </li>
  `).join("") || "<li>No tool steps returned.</li>";
  const citations = evidence.citations.map((citation) => `
    <li>
      <strong>${escapeHtml(citation.title || "Retrieved factory document")}</strong>
      ${citation.source ? ` · ${escapeHtml(citation.source)}` : ""}
      ${citation.excerpt ? `<p>${escapeHtml(citation.excerpt)}</p>` : ""}
    </li>
  `).join("") || "<li>No citations returned.</li>";
  return `
    <div class="run-summary">
      <span>Thread: ${escapeHtml(evidence.thread_id)}</span>
      <span>Run: ${escapeHtml(evidence.run_id)}</span>
      <span>Agent: ${escapeHtml(evidence.agent_id)}</span>
      <span>Status: ${escapeHtml(evidence.status)}</span>
    </div>
    <div class="agentic-grid">
      <div><h3>Tools used</h3><ul>${steps}</ul></div>
      <div><h3>Citations</h3><ul>${citations}</ul></div>
    </div>
  `;
}

async function runScenario() {
  el("run").disabled = true;
  el("notice").textContent = "Running analysis…";
  resetAction();
  try {
    const response = await fetch("/api/chat", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        scenario_id: el("scenario").value,
        target: el("target").value,
        messages: [{ role: "user", content: el("prompt").value }],
        allow_fallback: false,
      }),
    });
    const body = await response.json();
    if (!response.ok) throw new Error(body.detail || "Request failed");
    el("fast-response").className = "response";
    el("slow-response").className = "response";
    el("fast-response").innerHTML = responseHtml(body.fast_response);
    el("slow-response").innerHTML = responseHtml(body.slow_response);
    el("evidence").className = "evidence";
    el("evidence").innerHTML = evidenceHtml(body);
    el("agentic-evidence").className = "";
    el("agentic-evidence").innerHTML = agenticEvidenceHtml(body);
    state.currentRun = body;
    showActionControls(body);
    el("notice").textContent = `${body.mode} completed. No machine action has been executed.`;
  } catch (error) {
    el("notice").textContent = `Error: ${error.message}`;
  } finally {
    el("run").disabled = false;
  }
}

function resetAction() {
    state.currentRun = null;
    state.currentAction = null;
    el("action-controls").hidden = true;
    el("action-approval").hidden = true;
    el("action-result").textContent = "";
    el("approval-confirmed").checked = false;
    el("approve-action").disabled = true;
    el("action-explanation").textContent =
      "Run Robot 17 through the live Device and grounded Agentic Retrieval Edge path to create an evidence-bound action request.";
  }

function showActionControls(run) {
    const eligible =
      el("scenario").value === "robot-bearing-alarm" &&
      run.snapshot &&
      run.fast_response?.success &&
      !run.fast_response?.mock &&
      run.slow_response?.success &&
      !run.slow_response?.mock &&
      run.slow_response?.agentic_evidence?.status === "completed" &&
      run.slow_response?.agentic_evidence?.citations?.length > 0 &&
      run.slow_response?.agentic_evidence?.steps?.some(
        (step) =>
          step.step_type === "mcp_call" ||
          step.step_type === "tool_calls" ||
          step.step_type === "local_retrieval"
      ) &&
      run.guidance_status === "current";
    if (!eligible) return;
    el("action-explanation").textContent =
      `The current Fast + Slow result for ${run.snapshot.machine_event.machine_id} can be used to request a bounded simulated speed reduction.`;
    el("action-controls").hidden = false;
  }

async function createAction() {
    const run = state.currentRun;
    if (!run?.fast_response?.request_id) return;
    el("create-action").disabled = true;
    el("action-result").textContent = "Creating action request…";
    try {
      const response = await fetch("/api/actions/reduce-speed/requests", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          scenario_id: el("scenario").value,
          inference_request_id: run.fast_response.request_id,
          reduction_percent: Number(el("reduction-percent").value),
        }),
      });
      const body = await response.json();
      if (!response.ok) throw new Error(body.detail || "Action request failed");
      state.currentAction = body;
      el("action-summary").innerHTML = `
        <strong>Pending approval</strong>
        <p>Request a ${body.reduction_percent}% speed reduction for ${escapeHtml(body.machine_id)}.</p>
        <p class="muted">Action ID: ${escapeHtml(body.action_id)} · Connector: ${escapeHtml(body.connector_id)}</p>
      `;
      el("action-controls").hidden = true;
      el("action-approval").hidden = false;
      el("action-result").textContent = "";
    } catch (error) {
      el("action-result").textContent = `Error: ${error.message}`;
    } finally {
      el("create-action").disabled = false;
    }
  }

async function approveAction() {
    if (!state.currentAction || !el("approval-confirmed").checked) return;
    el("approve-action").disabled = true;
    el("action-result").textContent = "Executing approved simulated action…";
    try {
      const response = await fetch(`/api/actions/${state.currentAction.action_id}/approve`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          approved: true,
          operator_id: el("operator-id").value,
        }),
      });
      const body = await response.json();
      if (!response.ok) throw new Error(body.detail || "Action approval failed");
      state.currentAction = body;
      el("action-approval").hidden = true;
      el("action-result").textContent =
        `${body.outcome}. Approved by ${body.approved_by}.`;
    } catch (error) {
      el("action-result").textContent = `Error: ${error.message}`;
      el("approve-action").disabled = false;
  }
}

async function runPreflight() {
  el("preflight").disabled = true;
  el("preflight-status").textContent = "Checking all enabled environments…";
  try {
    const response = await fetch("/api/preflight", { method: "POST" });
    const body = await response.json();
    if (!response.ok) throw new Error(body.detail || "Environment check failed");
    el("preflight-status").textContent = body.message;
    const statusById = Object.fromEntries(body.providers.map((item) => [item.provider_id, item]));
    document.querySelectorAll("[data-provider]").forEach((node) => {
      const status = statusById[node.dataset.provider];
      const provider = state.providers.find((item) => item.provider_id === node.dataset.provider);
      let stateName = "Not checked";
      let detail = "";
      if (provider && !provider.enabled) {
        stateName = "Disabled";
        detail = "Provider is intentionally disabled";
      } else if (status && !status.configured) {
        stateName = "Not configured";
        detail = status.message;
      } else if (status && status.reachable) {
        stateName = "Ready";
        detail = status.message;
      } else if (status) {
        stateName = "Unavailable";
        detail = status.message;
      }

      node.querySelector(".provider-status").textContent =
        detail ? `${stateName} — ${detail}` : stateName;
      node.dataset.status = stateName.toLowerCase().replace(" ", "-");
    });
  } catch (error) {
    el("preflight-status").textContent = `Environment check failed: ${error.message}`;
  } finally {
    el("preflight").disabled = false;
  }
}

async function runGuidedScenario(scenarioId, button) {
  const card = button.closest("article");
  if (scenarioId === "management-brief") {
    sessionStorage.removeItem("governed-floor-management-preview");
    sessionStorage.removeItem("governed-floor-management-brief");
    card.classList.add("scenario-complete");
    button.textContent = "Opening report builder…";
    button.disabled = true;
    el("guided-demo-status").innerHTML =
      'Opening the scoped management-report workflow. <a href="/management">Review report scope</a>…';
    window.setTimeout(() => window.location.assign("/management"), 500);
    return;
  }
  const destination =
    scenarioId === "routine-local" || scenarioId === "network-loss"
      ? "/machine"
      : "/operations";
  button.disabled = true;
  button.textContent =
    scenarioId === "routine-local" ? "Running local agent…" : "Running factory flow…";
  card.classList.add("scenario-running");
  card.classList.remove("scenario-complete", "scenario-failed");
  el("guided-demo-status").textContent =
    "The scenario is running. This can take up to a minute when the factory agent is involved.";
  try {
    const response = await fetch(`/api/demo/scenarios/${scenarioId}/run`, { method: "POST" });
    const body = await response.json();
    if (!response.ok) throw new Error(body.detail || "Guided scenario failed");
    card.classList.remove("scenario-running");
    card.classList.add("scenario-complete");
    button.textContent = "Completed — opening result";
    el("guided-demo-status").innerHTML =
      `Scenario completed. Opening <a href="${destination}">${
        destination === "/machine"
          ? "Machine HMI"
          : "Governed Floor"
      }</a>…`;
    await refreshFabricStatus();
    window.setTimeout(() => {
      window.location.assign(destination);
    }, 700);
  } catch (error) {
    card.classList.remove("scenario-running");
    card.classList.add("scenario-failed");
    button.textContent = "Try again";
    el("guided-demo-status").textContent = `Scenario failed: ${error.message}`;
    button.disabled = false;
  }
}

async function resetGuidedDemo() {
  el("reset-guided-demo").disabled = true;
  try {
    const response = await fetch("/api/demo/reset", { method: "POST" });
    if (!response.ok) throw new Error("Reset failed");
    el("guided-demo-status").textContent = "Guided demo incidents cleared.";
    await Promise.all([refreshFactoryConnectivity(), refreshFabricStatus()]);
  } catch (error) {
    el("guided-demo-status").textContent = error.message;
  } finally {
    el("reset-guided-demo").disabled = false;
  }
}

function renderFactoryConnectivity(connected) {
  state.factoryConnected = connected;
  el("factory-connectivity-label").textContent =
    connected ? "Factory cluster connected" : "Factory cluster offline";
  el("factory-connectivity-badge").textContent = connected ? "Connected" : "Offline";
  el("factory-connectivity-badge").className =
    `badge ${connected ? "success" : "failure"}`;
  el("disconnect-factory").disabled = !connected;
  el("reconnect-factory").disabled = connected;
}

async function refreshFactoryConnectivity() {
  const response = await fetch("/api/demo/connectivity");
  const body = await response.json();
  if (!response.ok) throw new Error(body.detail || "Could not read factory connectivity");
  renderFactoryConnectivity(body.connected);
}

function renderFabricStatus(status) {
  const badge = el("fabric-publication-badge");
  const label = el("fabric-publication-label");
  const detail = el("fabric-publication-detail");
  const counts = el("fabric-publication-counts");
  counts.textContent =
    `${status.pending} pending · ${status.published} published · ${status.failed} failed`;
  if (!status.enabled) {
    label.textContent = "Local brief active; Fabric export disabled";
    detail.textContent =
      "Incidents remain available for the existing governed local brief. Fabric can be enabled independently.";
    badge.textContent = "Disabled";
    badge.className = "badge preview";
    return;
  }
  if (status.failed > 0) {
    label.textContent = "Fabric publication needs attention";
    detail.textContent = status.last_error || "One or more governed events could not be published.";
    badge.textContent = "Failed";
    badge.className = "badge failure";
    return;
  }
  label.textContent = "Publishing governed events to Fabric";
  detail.textContent =
    `Factory ${status.factory_id} publishes sanitized lifecycle events without affecting local control or reporting.`;
  badge.textContent = status.pending > 0 ? "Synchronizing" : "Connected";
  badge.className = `badge ${status.pending > 0 ? "preview" : "success"}`;
}

async function refreshFabricStatus() {
  try {
    const response = await fetch("/api/demo/fabric/status");
    const body = await response.json();
    if (!response.ok) throw new Error(body.detail || "Could not read Fabric publication status");
    renderFabricStatus(body);
  } catch (error) {
    el("fabric-publication-label").textContent = "Fabric publication status unavailable";
    el("fabric-publication-detail").textContent = error.message;
    el("fabric-publication-badge").textContent = "Unavailable";
    el("fabric-publication-badge").className = "badge failure";
    el("fabric-publication-counts").textContent = "";
  }
}

async function setFactoryConnectivity(connected) {
  const button = connected ? el("reconnect-factory") : el("disconnect-factory");
  button.disabled = true;
  button.textContent = connected ? "Synchronizing queued incidents…" : "Disconnecting…";
  el("guided-demo-status").textContent = connected
    ? "Factory connection restored. Synchronizing queued incidents through the slow agent…"
    : "Taking the factory connection offline. The machine-local agent remains available.";
  try {
    const target = connected ? "connected" : "offline";
    const response = await fetch(`/api/demo/connectivity/${target}`, { method: "POST" });
    const body = await response.json();
    if (!response.ok) throw new Error(body.detail || "Connectivity change failed");
    renderFactoryConnectivity(body.connected);
    const synchronized = body.synchronized?.length || 0;
    el("guided-demo-status").innerHTML = connected
      ? `Factory connected. ${synchronized} queued incident${synchronized === 1 ? "" : "s"} synchronized. <a href="/operations">Open Governed Floor</a>.`
      : 'Factory offline. Run <strong>Network loss and recovery</strong> to create a queued incident.';
  } catch (error) {
    el("guided-demo-status").textContent = `Connectivity change failed: ${error.message}`;
    await Promise.all([refreshFactoryConnectivity(), refreshFabricStatus()]);
  } finally {
    el("disconnect-factory").textContent = "Disconnect factory";
    el("reconnect-factory").textContent = "Reconnect and sync";
  }
}

async function initialize() {
  const [scenariosResponse, providersResponse] = await Promise.all([
    fetch("/api/scenarios"),
    fetch("/api/providers"),
  ]);
  state.scenarios = await scenariosResponse.json();
  state.providers = await providersResponse.json();
  const edgeProvider = state.providers.find(
    (provider) => provider.enabled && !provider.mock && provider.target_type === "edge"
  );
  const cloudProvider = state.providers.find(
    (provider) => provider.enabled && !provider.mock && provider.target_type === "cloud"
  );
  const foundryFallback = edgeProvider?.provider_id === "edge" &&
    edgeProvider?.capabilities?.shared_endpoint &&
    cloudProvider?.enabled;
  if (foundryFallback) {
    el("slow-architecture-title").textContent = "Foundry grounded model";
    el("slow-architecture-detail").textContent = "Local manual retrieval + shared inference";
    el("expert-architecture-title").textContent = "Foundry expert model";
    el("expert-architecture-detail").textContent = "Approved fleet analysis";
  }
  const enabledTargets = new Set(
    state.providers.filter((provider) => provider.enabled).map((provider) => provider.target_type)
  );
  state.scenarios = state.scenarios.filter((scenario) => {
    if (scenario.recommended_target === "device") return enabledTargets.has("device");
    if (scenario.recommended_target === "edge") return enabledTargets.has("edge");
    if (scenario.recommended_target === "cloud") return enabledTargets.has("cloud");
    return enabledTargets.has("device");
  });
  el("scenario").innerHTML = state.scenarios
    .map((scenario) => `<option value="${escapeHtml(scenario.id)}">${escapeHtml(scenario.title)}</option>`)
    .join("");
  const targetOrder = { device: 0, edge: 1, cloud: 2 };
  const diagnosticProviders = state.providers
    .filter((provider) => !provider.mock)
    .sort((left, right) => targetOrder[left.target_type] - targetOrder[right.target_type]);
  el("providers").innerHTML = diagnosticProviders.map((provider) => {
    const capabilities = Object.entries(provider.capabilities)
      .filter(([, enabled]) => enabled)
      .map(([name]) => name.replaceAll("_", " "))
      .join(", ") || "Base chat only";
    const initialStatus = !provider.enabled
      ? "Disabled — Provider is intentionally disabled"
      : provider.configured
        ? "Not checked — Select Check environments"
        : "Not configured — Required settings are missing";
    return `
      <article class="provider-card" data-provider="${escapeHtml(provider.provider_id)}"
        data-status="${provider.enabled ? (provider.configured ? "configured" : "not-configured") : "disabled"}">
        <strong>${escapeHtml(provider.provider_id)}</strong>
        <span>${escapeHtml(provider.target_type)}</span>
        <small>${escapeHtml(capabilities)}</small>
        <div class="provider-status">${escapeHtml(initialStatus)}</div>
      </article>
    `;
  }).join("");
  await refreshFactoryConnectivity();
  scenarioChanged();
}

el("scenario").addEventListener("change", scenarioChanged);
el("run").addEventListener("click", runScenario);
el("preflight").addEventListener("click", runPreflight);
el("create-action").addEventListener("click", createAction);
el("approval-confirmed").addEventListener("change", () => {
  el("approve-action").disabled = !el("approval-confirmed").checked;
});
el("approve-action").addEventListener("click", approveAction);
document.querySelectorAll("[data-guided-scenario]").forEach((button) => {
  button.addEventListener("click", () => runGuidedScenario(button.dataset.guidedScenario, button));
});
el("reset-guided-demo").addEventListener("click", resetGuidedDemo);
el("disconnect-factory").addEventListener("click", () => setFactoryConnectivity(false));
el("reconnect-factory").addEventListener("click", () => setFactoryConnectivity(true));
initialize().catch((error) => {
  el("notice").textContent = `Initialization failed: ${error.message}`;
});
