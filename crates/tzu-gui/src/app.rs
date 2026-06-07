use leptos::prelude::*;
use leptos_meta::{Stylesheet, Title, provide_meta_context};
use leptos_router::{
    StaticSegment,
    components::{Route, Router, Routes},
};
use lucide_leptos::{Activity, Circle, CircleCheck, Database, Play, RefreshCw, Settings2};

#[cfg(feature = "ssr")]
use leptos::config::LeptosOptions;

#[cfg(feature = "ssr")]
pub fn shell(options: LeptosOptions) -> impl IntoView {
    use leptos_meta::MetaTags;

    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload
                    disable_watch=cfg!(not(feature = "dev-hot-reload"))
                    options=options.clone()
                />
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body>
                <App/>
                <script src="/static/app.js"></script>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/tzu-gui.css"/>
        <Title text="tzu"/>
        <Router>
            <Routes fallback=|| view! { <Workbench/> }.into_view()>
                <Route path=StaticSegment("") view=Workbench/>
            </Routes>
        </Router>
    }
}

#[component]
fn Workbench() -> impl IntoView {
    view! {
        <main class="tzu-workbench" data-tzu-root>
            <aside class="sidebar">
                <div class="sidebar-header">
                    <div class="brand">"tzu"</div>
                    <span class="pill" id="health-pill">"booting"</span>
                </div>

                <section class="section grow">
                    <div class="section-title">
                        <Activity size=16/>
                        <span>"Runs"</span>
                    </div>
                    <div id="run-list" class="list muted">"No run reports yet."</div>
                </section>
            </aside>

            <section class="workspace">
                <header class="topbar">
                    <div>
                        <div class="eyebrow">"Local planning harness"</div>
                        <h1 id="plan-title">"No current plan"</h1>
                    </div>
                    <div class="topbar-actions">
                        <button class="icon-btn" id="refresh-btn" type="button" aria-label="Refresh state" title="Refresh state">
                            <RefreshCw size=18/>
                        </button>
                        <button class="icon-btn" id="init-btn" type="button" aria-label="Initialize state" title="Initialize state">
                            <Database size=18/>
                        </button>
                        <button class="icon-btn" id="settings-btn" type="button" aria-label="Inspect configuration" title="Inspect configuration">
                            <Settings2 size=18/>
                        </button>
                    </div>
                </header>

                <form id="plan-form" class="plan-form">
                    <div
                        id="goal-input"
                        class="goal-editor"
                        contenteditable="true"
                        role="textbox"
                        aria-label="Goal"
                        data-placeholder="Enter a goal"
                        spellcheck="false"
                    ></div>
                    <input id="goal-value" name="goal" type="hidden"/>
                    <div class="domain-inline segmented" role="radiogroup" aria-label="Planning domain">
                        <label><input type="radio" name="domain" value="generic" checked/>"Generic"</label>
                        <label><input type="radio" name="domain" value="coding"/>"Coding"</label>
                    </div>
                    <div
                        id="mention-suggestion"
                        class="mention-suggestion hidden"
                        aria-live="polite"
                    >
                        <div id="mention-suggestion-list" class="mention-suggestion-list"></div>
                    </div>
                    <button type="submit">
                        <CircleCheck size=17/>
                        <span>"Plan"</span>
                    </button>
                </form>

                <div class="summary-grid">
                    <StatCard label="Tasks" value_id="task-count" value="0"/>
                    <StatCard label="Candidates" value_id="candidate-count" value="0"/>
                    <StatCard label="Frontier" value_id="frontier-count" value="0"/>
                    <StatCard label="Champion" value_id="champion-id" value="none"/>
                    <StatCard label="Repo files" value_id="repo-files" value="0"/>
                    <StatCard label="Dirty" value_id="repo-dirty" value="unknown"/>
                </div>

                <section class="task-panel">
                    <div class="panel-heading">
                        <h2>"Plan Tasks"</h2>
                        <span class="muted" id="selected-task-status">"Select a task"</span>
                    </div>
                    <div id="task-list" class="task-list empty">
                        <Circle size=18/>
                        <span>"Create or load a plan to inspect tasks."</span>
                    </div>
                    <div class="task-detail-panel">
                        <div class="panel-heading compact">
                            <h2>"Selected Task"</h2>
                            <button class="run-btn" id="run-task-btn" type="button" disabled>
                                <Play size=15/>
                                <span>"Run"</span>
                            </button>
                        </div>
                        <div id="task-detail" class="detail-block muted">"No task selected."</div>
                        <div class="panel-heading compact">
                            <h2>"Acceptance"</h2>
                        </div>
                        <ul id="acceptance-list" class="acceptance-list"></ul>
                    </div>
                </section>

                <section class="report-panel">
                    <div class="panel-heading">
                        <h2>"Latest Report"</h2>
                    </div>
                    <pre id="latest-report" class="report">"No report yet."</pre>
                </section>
            </section>
        </main>
        <div id="toast-region" class="toast-region" aria-live="polite"></div>
        <div id="settings-dialog" class="settings-dialog hidden" aria-hidden="true">
            <div class="settings-dialog-backdrop" id="settings-dialog-backdrop"></div>
            <section
                class="settings-dialog-panel"
                role="dialog"
                aria-modal="true"
                aria-labelledby="settings-dialog-title"
                tabindex="-1"
            >
                <header class="settings-dialog-header">
                    <div>
                        <div class="eyebrow">"Configuration"</div>
                        <h2 id="settings-dialog-title">"tzu settings"</h2>
                    </div>
                    <button
                        class="icon-btn settings-dialog-close"
                        id="settings-dialog-close"
                        type="button"
                        aria-label="Close configuration"
                        title="Close"
                    >
                        "X"
                    </button>
                </header>
                <dl class="settings-grid">
                    <dt>"Project root"</dt>
                    <dd class="mono truncate" id="settings-project-root">"loading"</dd>
                    <dt>"Backend"</dt>
                    <dd class="mono truncate" id="backend-status">"checking"</dd>
                    <dt>"Config path"</dt>
                    <dd class="mono truncate" id="settings-config-path">"not configured"</dd>
                    <dt>"Project discovery"</dt>
                    <dd class="mono truncate" id="settings-projects-directories">"not configured"</dd>
                    <dt>"Nested repositories"</dt>
                    <dd class="mono truncate" id="settings-include-nested">"false"</dd>
                    <dt>"GUI"</dt>
                    <dd class="mono truncate" id="settings-gui">"loading"</dd>
                    <dt>"Discovered projects"</dt>
                    <dd id="settings-discovered-projects" class="settings-list muted">"None"</dd>
                </dl>
            </section>
        </div>
        <div id="error-dialog" class="error-dialog hidden" aria-hidden="true">
            <div class="error-dialog-backdrop" id="error-dialog-backdrop"></div>
            <section
                class="error-dialog-panel"
                role="dialog"
                aria-modal="true"
                aria-labelledby="error-dialog-title"
                tabindex="-1"
            >
                <header class="error-dialog-header">
                    <div>
                        <div class="eyebrow">"Error details"</div>
                        <h2 id="error-dialog-title">"Backend request failed"</h2>
                    </div>
                    <button
                        class="icon-btn error-dialog-close"
                        id="error-dialog-close"
                        type="button"
                        aria-label="Close error details"
                        title="Close"
                    >
                        "X"
                    </button>
                </header>
                <div id="error-dialog-explainer" class="error-explainer">
                    "No error details are available."
                </div>
                <pre id="error-dialog-logs" class="error-logs">"No logs captured."</pre>
            </section>
        </div>
    }
}

#[component]
fn StatCard(label: &'static str, value_id: &'static str, value: &'static str) -> impl IntoView {
    view! {
        <article class="stat-card">
            <span>{label}</span>
            <strong id=value_id>{value}</strong>
        </article>
    }
}
