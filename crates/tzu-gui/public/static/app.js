const state = {
  project: null,
  repo: null,
  selectedTaskId: null,
  latestError: null,
  config: null,
  configPath: null,
  discoveredProjects: [],
  contextReferences: [],
  mentionItems: [],
  selectedMentionIndex: 0,
  mentionResolveSerial: 0,
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
  if (status === 400) return "bad-request";
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
  setText("settings-project-root", project.project_root || "unknown");
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

function renderConfig(snapshot) {
  const config = snapshot?.config || snapshot || {};
  state.config = config;
  state.configPath = snapshot?.config_path || null;
  state.discoveredProjects = Array.isArray(snapshot?.discovered_projects)
    ? snapshot.discovered_projects
    : [];
  setText("settings-config-path", state.configPath || "not configured");
  const projectDirectories = [
    ...(config.projects_directory ? [config.projects_directory] : []),
    ...(Array.isArray(config.projects_directories) ? config.projects_directories : []),
  ];
  setText("settings-projects-directories", projectDirectories.length ? projectDirectories.join(", ") : "not configured");
  setText("settings-include-nested", config.include_nested_contexts ? "included" : "excluded");
  setText("settings-gui", `${config.gui?.host || "127.0.0.1"}:${config.gui?.port || 7070}`);
  renderDiscoveredProjectsSetting();
}

function renderContextReferences(references) {
  state.contextReferences = Array.isArray(references) ? references : [];
  updateMentionSuggestion();
}

function renderDiscoveredProjectsSetting() {
  const list = $("settings-discovered-projects");
  if (!list) return;
  if (!state.discoveredProjects.length) {
    list.classList.add("muted");
    list.textContent = "None";
    return;
  }
  list.classList.remove("muted");
  list.innerHTML = "";
  for (const project of state.discoveredProjects) {
    const row = document.createElement("div");
    row.className = "settings-list-row";
    row.textContent = `${project.name} (${project.path})`;
    list.appendChild(row);
  }
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
    const [health, project, repo, config, references] = await Promise.all([
      request("/api/health"),
      request("/api/state"),
      request("/api/repo"),
      request("/api/config"),
      request("/api/context-references"),
    ]);
    setText("backend-status", health.status);
    renderConfig(config);
    renderContextReferences(references);
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
  await resolveEditorMentions({ refocus: false });
  const goal = getEditorText().trim();
  if (!goal) return;
  const planGoal = buildPlanGoal();
  if (planGoal.error) {
    showToast(planGoal.error, "error");
    return;
  }
  try {
    const project = await request("/api/plans", {
      method: "POST",
      body: JSON.stringify({
        goal_display: planGoal.display,
        goal_raw: planGoal.raw,
        domain: domainValue(),
        context_roots: planGoal.contextRoots,
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

function buildPlanGoal() {
  const editor = $("goal-input");
  const contextRoots = [];
  const seen = new Set();
  let display = "";
  let raw = "";

  function appendNode(node) {
    if (node.nodeType === Node.TEXT_NODE) {
      display += node.textContent || "";
      raw += node.textContent || "";
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    if (node.classList.contains("goal-mention-chip")) {
      const label = node.dataset.display || node.textContent || "";
      if (node.dataset.status === "error") {
        return;
      }
      const path = node.dataset.path || "";
      display += label;
      raw += node.dataset.raw || label;
      if (path && !seen.has(path)) {
        seen.add(path);
        contextRoots.push(path);
      }
      return;
    }
    for (const child of node.childNodes) appendNode(child);
  }

  if (editor) {
    for (const node of editor.childNodes) appendNode(node);
  }

  const invalid = editor?.querySelector(".goal-mention-chip[data-status='error']");
  if (invalid) {
    return {
      display: display.trim(),
      raw: raw.trim(),
      contextRoots,
      error: invalid.dataset.error || `${invalid.dataset.display || "context path"} is invalid`,
    };
  }
  return { display: display.trim(), raw: raw.trim(), contextRoots, error: null };
}

function getEditorText() {
  const editor = $("goal-input");
  if (!editor) return "";
  let text = "";
  function append(node) {
    if (node.nodeType === Node.TEXT_NODE) {
      text += node.textContent || "";
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    if (node.classList.contains("goal-mention-chip")) {
      text += node.dataset.display || node.textContent || "";
      return;
    }
    if (node.tagName === "BR") {
      text += "\n";
      return;
    }
    for (const child of node.childNodes) append(child);
  }
  for (const node of editor.childNodes) append(node);
  return text.replace(/\u00a0/g, " ");
}

function syncGoalValue() {
  const hidden = $("goal-value");
  if (hidden) hidden.value = getEditorText();
}

function resolveMention(mention) {
  return state.contextReferences.find((reference) => reference.display === mention) || null;
}

function hasConfiguredProjectRoots() {
  return Boolean(
    state.config?.projects_directory
      || (Array.isArray(state.config?.projects_directories) && state.config.projects_directories.length)
  );
}

async function resolveEditorMentions(options = {}) {
  const editor = $("goal-input");
  if (!editor) return;
  const serial = ++state.mentionResolveSerial;
  const text = getEditorText();
  const tokens = text.match(/(@\S+|\s+|[^@\s]+|@)/g) || [];
  const absoluteMentions = [...new Set(tokens
    .filter((token) => token.startsWith("@/") && token.length > 1)
    .map((token) => token.slice(1)))];
  const absoluteResults = new Map();
  if (absoluteMentions.length) {
    try {
      const response = await request("/api/context-roots/resolve", {
        method: "POST",
        body: JSON.stringify({ paths: absoluteMentions }),
        operation: "Resolve context paths",
      });
      for (const result of response?.results || []) {
        absoluteResults.set(result.input, result);
      }
    } catch (error) {
      const captured = captureError(error, "Resolve context paths");
      showToast(captured.message, "error");
      return;
    }
  }
  if (serial !== state.mentionResolveSerial) return;

  const parts = tokens.map((token) => {
    if (token.startsWith("@/") && token.length > 1) {
      const input = token.slice(1);
      const result = absoluteResults.get(input);
      if (result?.ok) {
        return {
          kind: "chip",
          status: "ok",
          display: token,
          raw: `@${result.path}`,
          path: result.path,
          error: "",
        };
      }
      return {
        kind: "chip",
        status: "error",
        display: token,
        raw: token,
        path: "",
        error: result?.error || `context path \`${input}\` is unavailable`,
      };
    }
    const reference = resolveMention(token);
    if (reference) {
      return {
        kind: "chip",
        status: "ok",
        display: reference.display,
        raw: reference.raw,
        path: reference.path,
        error: "",
      };
    }
    return { kind: "text", text: token };
  });
  renderEditorParts(parts, options);
}

function renderEditorParts(parts, options = {}) {
  const editor = $("goal-input");
  if (!editor) return;
  editor.innerHTML = "";
  for (const part of parts) {
    if (part.kind === "chip") {
      editor.appendChild(createMentionChip(part));
    } else {
      editor.appendChild(document.createTextNode(part.text));
    }
  }
  syncGoalValue();
  if (options.refocus !== false) {
    placeCaretAtEnd(editor);
  }
}

function createMentionChip(part) {
  const chip = document.createElement("span");
  chip.className = `goal-mention-chip ${part.status}`;
  chip.contentEditable = "false";
  chip.tabIndex = 0;
  chip.textContent = part.display;
  chip.dataset.status = part.status;
  chip.dataset.display = part.display;
  chip.dataset.raw = part.raw;
  chip.dataset.path = part.path || "";
  chip.dataset.error = part.error || "";
  chip.title = part.status === "error" ? "Hover to show error. Ctrl-click to copy." : part.path;
  chip.addEventListener("click", (event) => {
    if (part.status === "error" && event.ctrlKey) {
      event.preventDefault();
      copyMentionError(chip);
      return;
    }
    expandMentionChip(chip);
  });
  chip.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      expandMentionChip(chip);
    }
  });
  chip.addEventListener("mouseenter", () => {
    if (chip.dataset.status === "error" && chip.dataset.error) {
      showToast(chip.dataset.error, "error");
    }
  });
  return chip;
}

async function copyMentionError(chip) {
  const text = chip.dataset.error || "";
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
    showToast("Copied error to clipboard.", "ok");
  } catch (_error) {
    showToast(text, "error");
  }
}

function expandMentionChip(chip) {
  const editor = $("goal-input");
  if (!editor || !chip.isConnected) return;
  const text = document.createTextNode(chip.dataset.display || chip.textContent || "");
  chip.replaceWith(text);
  placeCaretAfterNode(text);
  syncGoalValue();
  updateMentionSuggestion();
}

function placeCaretAfterNode(node) {
  const range = document.createRange();
  const selection = window.getSelection();
  range.setStartAfter(node);
  range.collapse(true);
  selection.removeAllRanges();
  selection.addRange(range);
  $("goal-input")?.focus();
}

function placeCaretAtEnd(node) {
  const range = document.createRange();
  const selection = window.getSelection();
  range.selectNodeContents(node);
  range.collapse(false);
  selection.removeAllRanges();
  selection.addRange(range);
  node.focus();
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

function openSettingsDialog() {
  const dialog = $("settings-dialog");
  const panel = document.querySelector(".settings-dialog-panel");
  if (!dialog) return;
  dialog.classList.remove("hidden");
  dialog.setAttribute("aria-hidden", "false");
  panel?.focus();
}

function closeSettingsDialog() {
  const dialog = $("settings-dialog");
  if (!dialog) return;
  dialog.classList.add("hidden");
  dialog.setAttribute("aria-hidden", "true");
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
    case "bad-request":
      return "The request was rejected before planning. Check that every referenced context path exists and points to a directory.";
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

function showToast(message, kind = "") {
  const region = $("toast-region");
  if (!region || !message) return;
  const toast = document.createElement("div");
  toast.className = `toast ${kind}`.trim();
  toast.textContent = message;
  region.appendChild(toast);
  window.setTimeout(() => {
    toast.remove();
  }, 4200);
}

function currentMentionQuery() {
  const editor = $("goal-input");
  const selection = window.getSelection();
  if (!editor || !selection?.rangeCount) return null;
  const range = selection.getRangeAt(0);
  if (!editor.contains(range.startContainer)) return null;
  const beforeRange = range.cloneRange();
  beforeRange.selectNodeContents(editor);
  beforeRange.setEnd(range.startContainer, range.startOffset);
  const before = beforeRange.toString().replace(/\u00a0/g, " ");
  const match = before.match(/@([^\s@]*)$/);
  if (!match) return null;
  const cursor = before.length;
  return {
    query: match[1].toLowerCase(),
    start: cursor - match[0].length,
    end: cursor,
  };
}

function updateMentionSuggestion() {
  const suggestion = $("mention-suggestion");
  const list = $("mention-suggestion-list");
  const current = currentMentionQuery();
  if (!suggestion || !list || !current || current.query.startsWith("/")) {
    hideMentionSuggestion();
    return;
  }
  if (!hasConfiguredProjectRoots()) {
    state.mentionItems = [];
    state.selectedMentionIndex = 0;
    list.innerHTML = "";
    const item = document.createElement("div");
    item.className = "mention-suggestion-message";
    item.textContent = "No project discovery directory is configured. Provide an absolute path like @/absolute/path.";
    list.appendChild(item);
    suggestion.classList.remove("hidden");
    return;
  }
  const items = state.contextReferences
    .filter((reference) => {
      const label = String(reference.label || "").toLowerCase();
      const display = String(reference.display || "").replace(/^@/, "").toLowerCase();
      const relative = String(reference.relative_path || "").toLowerCase();
      return label.startsWith(current.query) || display.startsWith(current.query) || relative.startsWith(current.query);
    })
    .slice(0, 8);
  state.mentionItems = items;
  state.selectedMentionIndex = Math.min(state.selectedMentionIndex, Math.max(items.length - 1, 0));
  if (!items.length) {
    hideMentionSuggestion();
    return;
  }
  list.innerHTML = "";
  items.forEach((reference, index) => {
    const item = document.createElement("button");
    item.type = "button";
    item.className = `mention-suggestion-item ${index === state.selectedMentionIndex ? "active" : ""}`;
    item.textContent = reference.display;
    item.title = reference.path;
    item.addEventListener("mousedown", (event) => event.preventDefault());
    item.addEventListener("click", () => insertMention(reference));
    list.appendChild(item);
  });
  suggestion.classList.remove("hidden");
}

function insertMention(reference) {
  const current = currentMentionQuery();
  if (!current) return;
  const text = getEditorText();
  renderEditorParts([
    {
      kind: "text",
      text: `${text.slice(0, current.start)}${reference.display} ${text.slice(current.end)}`,
    },
  ]);
  hideMentionSuggestion();
  resolveEditorMentions();
}

function acceptSelectedMention() {
  const reference = state.mentionItems[state.selectedMentionIndex];
  if (reference) insertMention(reference);
}

function moveMentionSelection(delta) {
  if (!state.mentionItems.length) return;
  state.selectedMentionIndex = (state.selectedMentionIndex + delta + state.mentionItems.length) % state.mentionItems.length;
  updateMentionSuggestion();
}

function hideMentionSuggestion() {
  state.mentionItems = [];
  state.selectedMentionIndex = 0;
  $("mention-suggestion")?.classList.add("hidden");
}

// --- Arena module ---
let arenaModule = null;
let arenaSpeed = 1.0;

async function initArena() {
  const canvas = $("tzu-arena-canvas");
  if (!canvas) return;
  try {
    const mod = await import("/static/tzu-arena/tzu_arena.js");
    await mod.default();
    arenaModule = mod;
    updateArenaVisibility();
    console.log("tzu-arena WASM module loaded");
  } catch (e) {
    console.warn("Arena module unavailable:", e);
    arenaModule = null;
  }
}

function updateArenaVisibility() {
  const section = $("arena-section");
  const placeholder = $("arena-placeholder");
  const plan = currentPlan();
  if (!section || !placeholder) return;
  const hasHarness = Boolean(plan?.harness?.candidates?.length);
  section.classList.toggle("hidden", !hasHarness);
  placeholder.classList.toggle("hidden", hasHarness);
}

function sendPlanToArena(plan) {
  if (!arenaModule || !plan?.harness) return;
  const json = JSON.stringify(plan.harness);
  try {
    arenaModule.set_arena_data(json);
  } catch (e) {
    console.warn("Failed to send plan to arena:", e);
  }
}

function setArenaSpeed(factor) {
  arenaSpeed = factor;
  const btn = $("arena-speed-btn");
  if (btn) btn.textContent = `${factor}×`;
  if (arenaModule) {
    try { arenaModule.set_arena_speed(factor); } catch (_) {}
  }
}

function skipArenaToResult() {
  if (arenaModule) {
    try { arenaModule.skip_to_result(); } catch (_) {}
  }
}

// Extend renderProject to show/hide arena
const _origRenderProject = renderProject;
renderProject = function(project) {
  _origRenderProject(project);
  updateArenaVisibility();
  sendPlanToArena(currentPlan());
};

document.addEventListener("DOMContentLoaded", () => {
  $("refresh-btn")?.addEventListener("click", refreshAll);
  $("init-btn")?.addEventListener("click", initialize);
  $("settings-btn")?.addEventListener("click", openSettingsDialog);
  $("plan-form")?.addEventListener("submit", createPlan);
  $("run-task-btn")?.addEventListener("click", runSelectedTask);
  $("goal-input")?.addEventListener("input", () => {
    syncGoalValue();
    updateMentionSuggestion();
  });
  $("goal-input")?.addEventListener("keyup", updateMentionSuggestion);
  $("goal-input")?.addEventListener("click", updateMentionSuggestion);
  $("goal-input")?.addEventListener("blur", () => {
    hideMentionSuggestion();
    resolveEditorMentions({ refocus: false });
  });
  $("goal-input")?.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown" && state.mentionItems.length) {
      event.preventDefault();
      moveMentionSelection(1);
    } else if (event.key === "ArrowUp" && state.mentionItems.length) {
      event.preventDefault();
      moveMentionSelection(-1);
    } else if ((event.key === "Enter" || event.key === "Tab") && state.mentionItems.length) {
      event.preventDefault();
      acceptSelectedMention();
    } else if (event.key === "Escape") {
      hideMentionSuggestion();
      closeSettingsDialog();
    } else if (event.key === " ") {
      window.setTimeout(() => {
        hideMentionSuggestion();
        resolveEditorMentions();
      }, 0);
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
  $("settings-dialog-close")?.addEventListener("click", closeSettingsDialog);
  $("settings-dialog-backdrop")?.addEventListener("click", closeSettingsDialog);
  document.querySelector(".settings-dialog-panel")?.addEventListener("click", (event) => {
    event.stopPropagation();
  });
  $("error-dialog-close")?.addEventListener("click", closeErrorDialog);
  $("error-dialog-backdrop")?.addEventListener("click", closeErrorDialog);
  document.querySelector(".error-dialog-panel")?.addEventListener("click", (event) => {
    event.stopPropagation();
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      closeErrorDialog();
      closeSettingsDialog();
    }
  });

  // Arena event listeners
  window.addEventListener("tzu-arena:fighter-click", (e) => {
    const candidateId = e.detail;
    if (!candidateId) return;
    const plan = currentPlan();
    const candidate = plan?.harness?.candidates?.find(c => c.id === candidateId);
    if (candidate) {
      setText("selected-task-status", `Candidate: ${candidateId}`);
      const detail = $("task-detail");
      if (detail) {
        detail.textContent = `Summary: ${candidate.candidate?.summary || "N/A"}\nTasks: ${candidate.candidate?.tasks?.length || 0}\nVerifier: ${candidate.score?.verifier_strength || "unknown"}\nRisk: ${candidate.score?.risk_profile || "unknown"}\nCost: ${candidate.score?.cost_tier || "unknown"}`;
      }
    }
  });

  window.addEventListener("tzu-arena:state-change", (e) => {
    console.log("Arena state:", e.detail);
  });

  window.addEventListener("tzu-arena:complete", (e) => {
    console.log("Arena champion:", e.detail);
    const championId = e.detail;
    if (championId) {
      setText("champion-id", championId);
    }
  });

  $("arena-speed-btn")?.addEventListener("click", () => {
    const speeds = [0.5, 1.0, 1.5, 2.0];
    const idx = speeds.indexOf(arenaSpeed);
    setArenaSpeed(speeds[(idx + 1) % speeds.length]);
  });

  $("arena-skip-btn")?.addEventListener("click", skipArenaToResult);

  initArena();
  refreshAll();
});
