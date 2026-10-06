const state = { preview: null, brief: null };
const el = (id) => document.getElementById(id);

function display(value) {
  return String(value ?? "").replaceAll("_", " ");
}

function dateTimeLocalValue(date) {
  const adjusted = new Date(date.getTime() - date.getTimezoneOffset() * 60000);
  return adjusted.toISOString().slice(0, 16);
}

function setRollingRange(hours) {
  const to = new Date();
  const from = new Date(to.getTime() - hours * 60 * 60 * 1000);
  el("range-from").value = dateTimeLocalValue(from);
  el("range-to").value = dateTimeLocalValue(to);
}

function renderList(id, items) {
  el(id).replaceChildren(
    ...items.map((item) => {
      const node = document.createElement("li");
      node.textContent = item;
      return node;
    })
  );
}

function renderPreview(preview) {
  state.preview = preview;
  sessionStorage.setItem("governed-floor-management-preview", JSON.stringify(preview));
  el("preview-section").hidden = false;
  el("preview-count").textContent = preview.incident_count;
  el("preview-target").textContent = display(preview.policy.target);
  el("preview-role").textContent = preview.policy.model_role;
  el("preview-id").textContent = preview.preview_id;
  el("approved-payload").textContent = JSON.stringify(preview.payload, null, 2);
  renderList("payload-exclusions", preview.exclusions);
  el("preview-section").scrollIntoView({ behavior: "smooth", block: "start" });
}

function renderBrief(brief) {
  state.brief = brief;
  sessionStorage.setItem("governed-floor-management-brief", JSON.stringify(brief));
  el("brief-report").hidden = false;
  el("brief-title").textContent = brief.title;
  el("brief-summary").textContent = brief.executive_summary;
  el("brief-generated").textContent =
    `Generated ${new Date(brief.generated_at).toLocaleString()}`;
  el("incident-count").textContent = brief.summary.incident_count;
  el("line-count").textContent = brief.summary.line_count;
  el("executed-count").textContent = brief.summary.status_counts.executed || 0;
  el("monitoring-count").textContent = brief.summary.status_counts.monitoring || 0;
  renderList("incident-outcomes", brief.incident_outcomes);
  renderList("recommended-follow-up", brief.recommended_follow_up);
  renderList("expert-recommendations", brief.expert_recommendations);
  el("expert-model").textContent = brief.governance.model_id;
  el("brief-classification").textContent = display(brief.governance.classification);
  el("brief-target").textContent = display(brief.governance.target);
  el("machine-authority").textContent =
    brief.governance.machine_authority ? "Enabled" : "None — advisory only";
  el("brief-preview-id").textContent = brief.governance.preview_id;
  el("generated-payload").textContent = JSON.stringify(brief.approved_payload, null, 2);
  el("brief-report").scrollIntoView({ behavior: "smooth", block: "start" });
}

async function previewBrief() {
  el("preview-brief").disabled = true;
  el("management-status").textContent = "Selecting incidents and constructing the approved payload locally…";
  el("brief-report").hidden = true;
  try {
    const from = new Date(el("range-from").value);
    const to = new Date(el("range-to").value);
    if (Number.isNaN(from.getTime()) || Number.isNaN(to.getTime())) {
      throw new Error("Choose a valid start and end time.");
    }
    const response = await fetch("/api/demo/management-brief/preview", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        from: from.toISOString(),
        to: to.toISOString(),
        mode: el("report-mode").value,
      }),
    });
    const body = await response.json();
    if (!response.ok) throw new Error(body.detail || "Could not preview report data");
    renderPreview(body);
    el("management-status").textContent =
      `${body.incident_count} incident${body.incident_count === 1 ? "" : "s"} selected. No cloud model has been called.`;
  } catch (error) {
    el("management-status").textContent = `Preview failed: ${error.message}`;
  } finally {
    el("preview-brief").disabled = false;
  }
}

async function generateBrief() {
  if (!state.preview) return;
  el("generate-brief").disabled = true;
  el("management-status").textContent =
    "Sending the exact approved preview to the cloud Expert…";
  try {
    const response = await fetch("/api/demo/management-brief/generate", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ preview_id: state.preview.preview_id }),
    });
    const body = await response.json();
    if (!response.ok) throw new Error(body.detail || "Could not generate management brief");
    renderBrief(body);
    el("management-status").textContent =
      "Expert analysis completed. No machine action was requested or executed.";
  } catch (error) {
    el("management-status").textContent = `Generation failed: ${error.message}`;
  } finally {
    el("generate-brief").disabled = false;
  }
}

el("range-preset").addEventListener("change", () => {
  if (el("range-preset").value === "rolling_24h") setRollingRange(24);
  if (el("range-preset").value === "last_7_days") setRollingRange(24 * 7);
});
el("range-from").addEventListener("input", () => { el("range-preset").value = "custom"; });
el("range-to").addEventListener("input", () => { el("range-preset").value = "custom"; });
el("preview-brief").addEventListener("click", previewBrief);
el("generate-brief").addEventListener("click", generateBrief);

setRollingRange(24);
const storedPreview = sessionStorage.getItem("governed-floor-management-preview");
const storedBrief = sessionStorage.getItem("governed-floor-management-brief");
try {
  if (storedPreview) renderPreview(JSON.parse(storedPreview));
  if (storedBrief) renderBrief(JSON.parse(storedBrief));
} catch {
  sessionStorage.removeItem("governed-floor-management-preview");
  sessionStorage.removeItem("governed-floor-management-brief");
}
