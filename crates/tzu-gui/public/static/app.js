const state = {
  project: null,
  repo: null,
  selectedTaskId: null,
};

const $ = (id) => document.getElementById(id);

async function request(path, options = {}) {
  const response = await fetch(path, {
    headers: { "content-type": "application/json", ...(options.headers || {}) },
    ...options,
  });
  const text = await response.text();
  const data = text ? JSON.parse(text) : null;
  if (!response.ok) {
    throw new Error(data?.error || `${response.status} ${response.statusText}`);
  }
  return data;
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
    const [health, project, repo] = await Promise.all([
      request("/api/health"),
      request("/api/state"),
      request("/api/repo"),
    ]);
    setText("backend-status", health.status);
    renderProject(project);
    renderRepo(repo);
    setHealth("ok", "online");
  } catch (error) {
    setText("backend-status", error.message);
    setHealth("error", "error");
  }
}

async function initialize() {
  const project = await request("/api/init", { method: "POST" });
  renderProject(project);
}

async function createPlan(event) {
  event.preventDefault();
  const goal = $("goal-input")?.value?.trim();
  if (!goal) return;
  const project = await request("/api/plans", {
    method: "POST",
    body: JSON.stringify({ goal, domain: domainValue() }),
  });
  state.selectedTaskId = null;
  renderProject(project);
}

async function runSelectedTask() {
  if (!state.selectedTaskId) return;
  const report = await request(`/api/tasks/${encodeURIComponent(state.selectedTaskId)}/run`, {
    method: "POST",
    body: JSON.stringify({ mode: "mock" }),
  });
  setText("latest-report", JSON.stringify(report, null, 2));
  await refreshAll();
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

document.addEventListener("DOMContentLoaded", () => {
  $("refresh-btn")?.addEventListener("click", refreshAll);
  $("init-btn")?.addEventListener("click", initialize);
  $("plan-form")?.addEventListener("submit", createPlan);
  $("run-task-btn")?.addEventListener("click", runSelectedTask);
  refreshAll();
});
