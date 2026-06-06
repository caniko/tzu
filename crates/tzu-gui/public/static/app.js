const state = {
  project: null,
  repo: null,
  selectedTaskId: null,
  latestError: null,
  config: null,
  discoveredProjects: [],
  suggestedProject: null,
  configApplied: false,
};

const $ = (id) => document.getElementById(id);

async function request(path, options = {}) {
  const { operation = "Backend request", ...fetchOptions } = options;
  const startedAt = new Date().toISOString();
  const response = await fetch(path, {
    headers: { "content-type": "application/json", ...(fetchOptions.headers || {}) },
    ...fetchOptions,
  });
  const text = await response.text();
  const data = parseResponseBody(text);
  if (!response.ok) {
    const error = new Error(data?.error || `${response.status} ${response.statusText}`);
    error.kind = data?.kind || classifyErrorKind(error.message, response.status);
    error.operation = operation;
    error.status = response.status;
    error.statusText = response.statusText;
    error.path = path;
    error.method = fetchOptions.method || "GET";
    error.responseBody = data || text;
    error.startedAt = startedAt;
    error.endedAt = new Date().toISOString();
    throw error;
  }
  return data;
}

function parseResponseBody(text) {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch (_error) {
    return text;
  }
}

function setText(id, value) {
  const node = $(id);
  if (node) node.textContent = value;
}

function setHealth(kind, text) {
  const pill = $("health-pill");
  if (!pill) return;
  pill.textContent = text;
  pill.className = `pill ${kind}`;
  updateErrorAffordance();
}

function clearLatestError() {
  state.latestError = null;
  updateErrorAffordance();
}

function captureError(error, operation) {
  const captured = normalizeError(error, operation);
  state.latestError = captured;
  setText("backend-status", captured.message);
  setHealth("error", "error");
  updateErrorAffordance();
  return captured;
}

function normalizeError(error, operation) {
  const message = error?.message || String(error || "Unknown error");
  const kind = normalizeErrorKind(error?.kind, message, error?.status);
  return {
    message,
    kind,
    operation: error?.operation || operation,
    timestamp: new Date().toISOString(),
    status: error?.status || null,
    statusText: error?.statusText || null,
    method: error?.method || null,
    path: error?.path || null,
    responseBody: error?.responseBody || null,
    stack: error?.stack || null,
  };
}

function normalizeErrorKind(kind, message, status) {
  if (kind === "runner-error") {
    const lower = String(message || "").toLowerCase();
    if (lower.includes("database")) return "database-unavailable";
    if (lower.includes("no current plan") || lower.includes("not found")) {
      return "missing-resource";
    }
  }
  return kind || classifyErrorKind(message, status);
}

function classifyErrorKind(message, status) {
  const lower = String(message || "").toLowerCase();
  if (lower.includes("database")) return "database-unavailable";
  if (lower.includes("no current plan") || lower.includes("not found") || status === 404) {
    return "missing-resource";
  }
  if (status && status >= 500) return "internal";
  return "network";
}

function updateErrorAffordance() {
  const hasError = Boolean(state.latestError);
  for (const node of [$("health-pill"), $("backend-status")]) {
    if (!node) continue;
    node.classList.toggle("error-clickable", hasError);
    node.setAttribute("aria-disabled", hasError ? "false" : "true");
    node.setAttribute("role", hasError ? "button" : "status");
    if (hasError) {
      node.setAttribute("tabindex", "0");
    } else {
      node.removeAttribute("tabindex");
    }
    node.title = hasError ? "Show error details" : "";
  }
}

function domainValue() {
  return document.querySelector("input[name='domain']:checked")?.value || "generic";
}

function contextRootsValue() {
  return ($("context-roots-input")?.value || "")
    .split(/\r?\n/)
    .map((value) => value.trim())
    .filter(Boolean);
}

function includeNestedContextsValue() {
  return Boolean($("include-nested-contexts")?.checked);
}

function currentPlan() {
  return state.project?.current_plan || null;
}

function orderedTasks(plan) {
  if (!plan) return [];
  const tasks = [...plan.tasks];
  const byId = new Map(tasks.map((task) => [task.id, task]));
  const rank = new Map();
  function visit(task) {
    if (rank.has(task.id)) return rank.get(task.id);
    const value = 1 + Math.max(0, ...task.depends_on.map((id) => byId.has(id) ? visit(byId.get(id)) : 0));
    rank.set(task.id, value);
    return value;
  }
  tasks.forEach(visit);
  return tasks.sort((a, b) => (rank.get(a.id) - rank.get(b.id)) || a.id.localeCompare(b.id));
}

function renderProject(project) {
  state.project = project;
  setText("project-root", project.project_root || "unknown");
  const plan = currentPlan();
  setText("plan-title", plan ? plan.goal : "No current plan");
  setText("task-count", plan ? String(plan.tasks.length) : "0");
  setText("candidate-count", plan?.harness ? String(plan.harness.candidates.length) : "0");
  setText("frontier-count", plan?.harness?.frontier ? String(plan.harness.frontier.retained_candidate_ids.length) : "0");
  setText("champion-id", plan?.harness?.frontier?.selected_candidate_id || plan?.harness?.selected_candidate_id || "none");
  renderTasks();
  renderReports();
}

function renderRepo(repo) {
  state.repo = repo;
  setText("repo-files", String(repo.files?.length || 0));
  setText("repo-dirty", repo.dirty ? "yes" : "no");
}

function renderConfig(config) {
  state.config = config || {};
  const nested = $("include-nested-contexts");
  if (nested && !state.configApplied) {
    nested.checked = Boolean(state.config?.include_nested_contexts);
  }
  state.configApplied = true;
}

function renderDiscoveredProjects(projects) {
  state.discoveredProjects = Array.isArray(projects) ? projects : [];
  const list = $("discovered-projects");
  if (!list) return;
  if (!state.discoveredProjects.length) {
    list.classList.add("muted");
    list.textContent = "No configured projects.";
    return;
  }
  list.classList.remove("muted");
  list.innerHTML = "";
  for (const project of state.discoveredProjects) {
    const item = document.createElement("button");
    item.type = "button";
    item.className = "project-chip";
    item.textContent = project.name;
    item.title = project.path;
    item.addEventListener("click", () => addContextRoot(project.path));
    list.appendChild(item);
  }
  updateProjectSuggestion();
}

function renderTasks() {
  const list = $("task-list");
  const plan = currentPlan();
  const tasks = orderedTasks(plan);
  if (!list) return;
  list.classList.remove("empty");
  list.innerHTML = "";
  if (!tasks.length) {
    list.classList.add("empty");
    list.innerHTML = "<span>Create or load a plan to inspect tasks.</span>";
    renderSelectedTask(null);
    return;
  }
  if (!tasks.some((task) => task.id === state.selectedTaskId)) {
    state.selectedTaskId = tasks[0].id;
  }
  for (const task of tasks) {
    const item = document.createElement("button");
    item.type = "button";
    item.className = `task-item ${task.id === state.selectedTaskId ? "active" : ""}`;
    item.dataset.taskId = task.id;
    item.innerHTML = `
      <span>
        <span class="task-title">${escapeHtml(task.title)}</span>
        <span class="task-id">${escapeHtml(task.id)}</span>
      </span>
      <span class="status ${task.status}">${escapeHtml(task.status)}</span>
    `;
    item.addEventListener("click", () => {
      state.selectedTaskId = task.id;
      renderTasks();
    });
    list.appendChild(item);
  }
  renderSelectedTask(tasks.find((task) => task.id === state.selectedTaskId));
}

function renderSelectedTask(task) {
  const runButton = $("run-task-btn");
  if (runButton) runButton.disabled = !task;
  setText("selected-task-status", task ? task.status : "Select a task");
  const detail = $("task-detail");
  if (detail) {
    detail.classList.toggle("muted", !task);
    detail.textContent = task ? task.description : "No task selected.";
  }
  const acceptance = $("acceptance-list");
  if (acceptance) {
    acceptance.innerHTML = "";
    for (const criterion of task?.acceptance_criteria || []) {
      const item = document.createElement("li");
      item.textContent = criterion.description;
      acceptance.appendChild(item);
    }
  }
}

function renderReports() {
  const reports = state.project?.run_reports || [];
  const list = $("run-list");
  if (list) {
    list.innerHTML = "";
    if (!reports.length) {
      list.classList.add("muted");
      list.textContent = "No run reports yet.";
    } else {
      list.classList.remove("muted");
      for (const report of reports.slice().reverse()) {
        const row = document.createElement("div");
        row.className = "run-report-row";
        row.textContent = `${report.task_id}: ${report.status}`;
        list.appendChild(row);
      }
    }
  }
  const latest = reports[reports.length - 1];
  setText("latest-report", latest ? JSON.stringify(latest, null, 2) : "No report yet.");
}

async function refreshAll() {
  try {
    setHealth("", "loading");
    const [health, project, repo, config, projects] = await Promise.all([
      request("/api/health"),
      request("/api/state"),
      request("/api/repo"),
      request("/api/config"),
      request("/api/projects"),
    ]);
    setText("backend-status", health.status);
    renderConfig(config);
    renderDiscoveredProjects(projects);
    renderProject(project);
    renderRepo(repo);
    clearLatestError();
    setHealth("ok", "online");
  } catch (error) {
    captureError(error, "Refresh project state");
  }
}

async function initialize() {
  try {
    const project = await request("/api/init", {
      method: "POST",
      operation: "Initialize project state",
    });
    clearLatestError();
    renderProject(project);
  } catch (error) {
    captureError(error, "Initialize project state");
  }
}

async function createPlan(event) {
  event.preventDefault();
  const goal = $("goal-input")?.value?.trim();
  if (!goal) return;
  try {
    const project = await request("/api/plans", {
      method: "POST",
      body: JSON.stringify({
        goal,
        domain: domainValue(),
        context_roots: contextRootsValue(),
        include_nested_contexts: includeNestedContextsValue(),
      }),
      operation: "Create plan",
    });
    clearLatestError();
    state.selectedTaskId = null;
    renderProject(project);
  } catch (error) {
    captureError(error, "Create plan");
  }
}

async function runSelectedTask() {
  if (!state.selectedTaskId) return;
  try {
    const report = await request(`/api/tasks/${encodeURIComponent(state.selectedTaskId)}/run`, {
      method: "POST",
      body: JSON.stringify({ mode: "mock" }),
      operation: "Run selected task",
    });
    clearLatestError();
    setText("latest-report", JSON.stringify(report, null, 2));
    await refreshAll();
  } catch (error) {
    captureError(error, "Run selected task");
  }
}

function openErrorDialog() {
  if (!state.latestError) return;
  const dialog = $("error-dialog");
  const panel = document.querySelector(".error-dialog-panel");
  if (!dialog) return;
  setText("error-dialog-title", `${state.latestError.operation || "Request"} failed`);
  setText("error-dialog-explainer", explainError(state.latestError));
  setText("error-dialog-logs", JSON.stringify(state.latestError, null, 2));
  dialog.classList.remove("hidden");
  dialog.setAttribute("aria-hidden", "false");
  panel?.focus();
}

function closeErrorDialog() {
  const dialog = $("error-dialog");
  if (!dialog) return;
  dialog.classList.add("hidden");
  dialog.setAttribute("aria-hidden", "true");
}

function explainError(error) {
  switch (error.kind) {
    case "database-unavailable":
    case "runner-error":
      if (String(error.message).toLowerCase().includes("database")) {
        return "The configured database could not be reached or initialized. Check the database URL, make sure the service is running, or switch to a local SQLite URL for development.";
      }
      return "The backend runner rejected the request. The details below include the operation, route, status, and raw backend response.";
    case "missing-resource":
      return "This action needs a current plan or task that does not exist in the loaded project state. Create or refresh the plan, then try again.";
    case "network":
      return "The GUI could not complete a backend request. The server may be unavailable, the request may have failed, or the response could not be processed.";
    case "internal":
    default:
      return "The backend returned an unexpected error. The raw request details below are preserved so the failure can be diagnosed.";
  }
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function currentGoalWord() {
  const input = $("goal-input");
  if (!input) return "";
  const cursor = input.selectionStart ?? input.value.length;
  const before = input.value.slice(0, cursor);
  const match = before.match(/[A-Za-z0-9_.-]+$/);
  return match ? match[0] : "";
}

function updateProjectSuggestion() {
  const suggestion = $("project-suggestion");
  if (!suggestion) return;
  const word = currentGoalWord().toLowerCase();
  const project = word.length >= 2
    ? state.discoveredProjects.find((project) => {
        const name = String(project.name || "").toLowerCase();
        return name === word || name.startsWith(word);
      })
    : null;
  state.suggestedProject = project || null;
  if (!project) {
    suggestion.classList.add("hidden");
    setText("project-suggestion-text", "");
    return;
  }
  setText("project-suggestion-text", `Add ${project.name} as context`);
  suggestion.classList.remove("hidden");
}

function addContextRoot(path) {
  if (!path) return;
  const input = $("context-roots-input");
  if (!input) return;
  const roots = contextRootsValue();
  if (!roots.includes(path)) {
    roots.push(path);
    input.value = roots.join("\n");
  }
  hideProjectSuggestion();
}

function acceptProjectSuggestion() {
  if (!state.suggestedProject) return;
  addContextRoot(state.suggestedProject.path);
}

function hideProjectSuggestion() {
  state.suggestedProject = null;
  $("project-suggestion")?.classList.add("hidden");
}

document.addEventListener("DOMContentLoaded", () => {
  $("refresh-btn")?.addEventListener("click", refreshAll);
  $("init-btn")?.addEventListener("click", initialize);
  $("plan-form")?.addEventListener("submit", createPlan);
  $("run-task-btn")?.addEventListener("click", runSelectedTask);
  $("goal-input")?.addEventListener("input", updateProjectSuggestion);
  $("goal-input")?.addEventListener("keyup", updateProjectSuggestion);
  $("goal-input")?.addEventListener("click", updateProjectSuggestion);
  $("goal-input")?.addEventListener("keydown", (event) => {
    if (event.ctrlKey && event.key === " ") {
      event.preventDefault();
      acceptProjectSuggestion();
    } else if (event.key === "Escape") {
      hideProjectSuggestion();
    }
  });
  $("health-pill")?.addEventListener("click", openErrorDialog);
  $("backend-status")?.addEventListener("click", openErrorDialog);
  for (const id of ["health-pill", "backend-status"]) {
    $(id)?.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        openErrorDialog();
      }
    });
  }
  $("error-dialog-close")?.addEventListener("click", closeErrorDialog);
  $("error-dialog-backdrop")?.addEventListener("click", closeErrorDialog);
  document.querySelector(".error-dialog-panel")?.addEventListener("click", (event) => {
    event.stopPropagation();
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") closeErrorDialog();
  });
  refreshAll();
});
