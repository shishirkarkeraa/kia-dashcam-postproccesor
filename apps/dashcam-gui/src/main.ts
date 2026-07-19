import "./styles.css";
import brandLogo from "./assets/White_PP.png";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";

type CleanupPolicy = "keep" | "trash";

interface VideoCandidate {
  id: string;
  path: string;
  displayPath: string;
  included: boolean;
  valid: boolean;
  reason?: string;
  recordingTime?: string;
  timestampSource?: string;
  videoStreams: number;
  audioStreams: number;
  durationSeconds?: number;
}

interface JobEvent {
  kind: "started" | "progress" | "warning" | "log" | "completed" | "cancelled";
  stage: string;
  message: string;
  currentFile?: string;
  completedTasks: number;
  totalTasks: number;
  elapsedSeconds: number;
  etaSeconds?: number;
}

interface JobResult {
  outputPath: string;
  processedFiles: number;
  warnings: string[];
}

interface UpdateInfo {
  version: string;
  notes?: string;
  date?: string;
}

interface PendingJobInfo {
  outputPath: string;
  inputCount: number;
  completedTasks: number;
  totalTasks: number;
}

const state = {
  candidates: [] as VideoCandidate[],
  outputDir: "",
  cleanup: "keep" as CleanupPolicy,
  running: false,
  scanning: false,
  dragActive: false,
  progress: null as JobEvent | null,
  logs: [] as string[],
  result: null as JobResult | null,
  error: "",
  pendingJob: null as PendingJobInfo | null,
};

document.querySelector<HTMLDivElement>("#app")!.innerHTML = `
  <main class="app-shell">
    <header class="topbar">
      <div class="brand">
        <img class="brand-logo" src="${brandLogo}" alt="Dashcam Postprocessing Engine" />
      </div>
      <button id="check-update" class="ghost-button" type="button">Check for updates</button>
    </header>

    <section class="workspace-grid">
      <div class="primary-column">
        <section id="drop-zone" class="drop-zone" tabindex="0" aria-label="Drop AVI files or folders">
          <div class="drop-icon" aria-hidden="true">＋</div>
          <div>
            <h2>Drop recordings or a folder</h2>
            <p>AVI files are scanned recursively and ordered by recording time.</p>
          </div>
          <div class="drop-actions">
            <button id="browse-folder" class="primary-button" type="button">Browse folder</button>
            <button id="browse-files" class="secondary-button" type="button">Choose files</button>
          </div>
        </section>

        <section class="panel candidates-panel">
          <div class="panel-heading">
            <div>
              <p class="eyebrow">FINAL SEQUENCE</p>
              <h2>Selected recordings</h2>
            </div>
            <div class="selection-tools">
              <span id="selection-count" class="count-chip">0 selected</span>
              <button id="select-valid" class="text-button" type="button">Select valid</button>
              <button id="clear-selection" class="text-button" type="button">Clear</button>
            </div>
          </div>
          <div class="table-wrap">
            <table>
              <thead>
                <tr>
                  <th class="check-column"><span class="sr-only">Include</span></th>
                  <th>Recording</th>
                  <th>Detected time</th>
                  <th>Streams</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody id="candidate-body">
                <tr class="empty-row"><td colspan="5">No recordings scanned yet.</td></tr>
              </tbody>
            </table>
          </div>
        </section>
      </div>

      <aside class="sidebar">
        <section class="panel settings-panel">
          <p class="eyebrow">JOB SETTINGS</p>
          <h2>Output and cleanup</h2>
          <label class="field-label" for="output-dir">Output folder</label>
          <div class="path-field">
            <input id="output-dir" type="text" placeholder="Choose an output folder" readonly />
            <button id="browse-output" class="icon-button" type="button" aria-label="Choose output folder">…</button>
          </div>
          <fieldset>
            <legend>After a clip is stacked</legend>
            <label class="radio-card">
              <input type="radio" name="cleanup" value="keep" checked />
              <span><strong>Keep originals</strong><small>Source AVI files remain in place.</small></span>
            </label>
            <label class="radio-card warning-option">
              <input type="radio" name="cleanup" value="trash" />
              <span><strong>Move to system Trash</strong><small>Only after that clip stacks successfully.</small></span>
            </label>
          </fieldset>
          <button id="start-job" class="start-button" type="button" disabled>
            <span>Process selected videos</span><span aria-hidden="true">→</span>
          </button>
          <p class="safety-note">Invalid files are skipped and never modified. Existing final videos are never overwritten.</p>
        </section>

        <section id="progress-panel" class="panel progress-panel hidden" aria-live="polite">
          <div class="panel-heading compact">
            <div>
              <p class="eyebrow">PROCESSING</p>
              <h2 id="progress-stage">Preparing</h2>
            </div>
            <span id="progress-percent" class="percent">0%</span>
          </div>
          <div class="progress-track"><div id="progress-bar" class="progress-bar"></div></div>
          <p id="progress-file" class="current-file">Waiting…</p>
          <div class="metrics">
            <div><span>Tasks</span><strong id="metric-tasks">0 / 0</strong></div>
            <div><span>Elapsed</span><strong id="metric-elapsed">00:00:00</strong></div>
            <div><span>ETA</span><strong id="metric-eta">Calculating…</strong></div>
          </div>
          <details>
            <summary>Processing log</summary>
            <pre id="job-log"></pre>
          </details>
          <button id="cancel-job" class="danger-button" type="button">Cancel safely</button>
        </section>

        <section id="result-panel" class="panel result-panel hidden" aria-live="polite">
          <div id="result-icon" class="result-icon">✓</div>
          <h2 id="result-title">Processing complete</h2>
          <p id="result-message"></p>
        </section>
      </aside>
    </section>
  </main>
`;

const elements = {
  dropZone: byId<HTMLElement>("drop-zone"),
  browseFolder: byId<HTMLButtonElement>("browse-folder"),
  browseFiles: byId<HTMLButtonElement>("browse-files"),
  candidateBody: byId<HTMLTableSectionElement>("candidate-body"),
  selectionCount: byId<HTMLElement>("selection-count"),
  selectValid: byId<HTMLButtonElement>("select-valid"),
  clearSelection: byId<HTMLButtonElement>("clear-selection"),
  outputDir: byId<HTMLInputElement>("output-dir"),
  browseOutput: byId<HTMLButtonElement>("browse-output"),
  startJob: byId<HTMLButtonElement>("start-job"),
  cancelJob: byId<HTMLButtonElement>("cancel-job"),
  checkUpdate: byId<HTMLButtonElement>("check-update"),
  progressPanel: byId<HTMLElement>("progress-panel"),
  progressStage: byId<HTMLElement>("progress-stage"),
  progressPercent: byId<HTMLElement>("progress-percent"),
  progressBar: byId<HTMLElement>("progress-bar"),
  progressFile: byId<HTMLElement>("progress-file"),
  metricTasks: byId<HTMLElement>("metric-tasks"),
  metricElapsed: byId<HTMLElement>("metric-elapsed"),
  metricEta: byId<HTMLElement>("metric-eta"),
  jobLog: byId<HTMLElement>("job-log"),
  resultPanel: byId<HTMLElement>("result-panel"),
  resultIcon: byId<HTMLElement>("result-icon"),
  resultTitle: byId<HTMLElement>("result-title"),
  resultMessage: byId<HTMLElement>("result-message"),
};

elements.browseFolder.addEventListener("click", async () => {
  const folder = await open({ directory: true, multiple: false, title: "Choose dashcam folder" });
  if (typeof folder === "string") {
    state.outputDir = folder;
    await scan([folder], folder);
  }
});

elements.browseFiles.addEventListener("click", async () => {
  const files = await open({
    directory: false,
    multiple: true,
    title: "Choose two-channel AVI recordings",
    filters: [{ name: "AVI recordings", extensions: ["avi", "AVI"] }],
  });
  if (Array.isArray(files) && files.length > 0) {
    state.outputDir = commonParent(files);
    await scan(files, undefined);
  }
});

elements.browseOutput.addEventListener("click", async () => {
  const folder = await open({ directory: true, multiple: false, title: "Choose output folder" });
  if (typeof folder === "string") {
    state.outputDir = folder;
    await refreshPendingJob();
    render();
  }
});

elements.selectValid.addEventListener("click", () => {
  state.candidates.forEach((candidate) => (candidate.included = candidate.valid));
  render();
});

elements.clearSelection.addEventListener("click", () => {
  state.candidates.forEach((candidate) => (candidate.included = false));
  render();
});

elements.candidateBody.addEventListener("change", (event) => {
  const input = event.target as HTMLInputElement;
  const candidate = state.candidates.find((item) => item.id === input.dataset.id);
  if (candidate?.valid) {
    candidate.included = input.checked;
    renderSummary();
  }
});

document.querySelectorAll<HTMLInputElement>('input[name="cleanup"]').forEach((input) => {
  input.addEventListener("change", () => {
    state.cleanup = input.value as CleanupPolicy;
  });
});

elements.startJob.addEventListener("click", startJob);
elements.cancelJob.addEventListener("click", async () => {
  elements.cancelJob.disabled = true;
  elements.cancelJob.textContent = "Cancelling…";
  await invoke("cancel_job");
});
elements.checkUpdate.addEventListener("click", () => checkForUpdate(true));

elements.dropZone.addEventListener("keydown", (event) => {
  if (event.key === "Enter" || event.key === " ") elements.browseFolder.click();
});

async function initializeNativeEvents(): Promise<void> {
  await listen<JobEvent>("job-event", ({ payload }) => {
    state.progress = payload;
    state.logs.push(`${payload.stage}: ${payload.message}${payload.currentFile ? ` — ${payload.currentFile}` : ""}`);
    renderProgress();
  });

  await getCurrentWebview().onDragDropEvent(async (event) => {
    if (event.payload.type === "over") {
      state.dragActive = true;
      elements.dropZone.classList.add("drag-active");
    } else if (event.payload.type === "leave") {
      state.dragActive = false;
      elements.dropZone.classList.remove("drag-active");
    } else if (event.payload.type === "drop") {
      state.dragActive = false;
      elements.dropZone.classList.remove("drag-active");
      const paths = event.payload.paths;
      if (paths.length > 0) {
        state.outputDir = inferredOutput(paths);
        await scan(paths, undefined);
      }
    }
  });
}

window.addEventListener("beforeunload", (event) => {
  if (state.running) {
    event.preventDefault();
  }
});

async function scan(paths: string[], displayRoot?: string): Promise<void> {
  if (state.running) return;
  state.scanning = true;
  state.error = "";
  state.result = null;
  render();
  try {
    state.candidates = await invoke<VideoCandidate[]>("scan_inputs", {
      paths,
      displayRoot: displayRoot ?? null,
    });
    await refreshPendingJob();
  } catch (error) {
    state.error = String(error);
    state.candidates = [];
  } finally {
    state.scanning = false;
    render();
  }
}

async function startJob(): Promise<void> {
  const selected = state.candidates.filter((candidate) => candidate.valid && candidate.included);
  const resumeOnly = selected.length === 0 && Boolean(state.pendingJob);
  if ((!resumeOnly && selected.length === 0) || !state.outputDir) return;
  const cleanupText = state.cleanup === "trash"
    ? "Originals will move to the system Trash after each successful stack."
    : "Originals will be kept.";
  const prompt = resumeOnly
    ? `Resume the interrupted ${state.pendingJob!.inputCount}-recording job?\n\nOutput: ${state.pendingJob!.outputPath}`
    : `Process ${selected.length} recording${selected.length === 1 ? "" : "s"}?\n\nOutput: ${state.outputDir}\n${cleanupText}`;
  const accepted = window.confirm(prompt);
  if (!accepted) return;

  state.running = true;
  state.error = "";
  state.result = null;
  state.logs = [];
  state.progress = null;
  render();
  try {
    state.result = await invoke<JobResult>("start_job", {
      plan: {
        candidates: state.candidates,
        outputDir: state.outputDir,
        cleanup: state.cleanup,
        restart: false,
        resumePending: resumeOnly,
      },
    });
  } catch (error) {
    state.error = String(error);
  } finally {
    state.running = false;
    await refreshPendingJob();
    elements.cancelJob.disabled = false;
    elements.cancelJob.textContent = "Cancel safely";
    render();
    void checkForUpdate(false);
  }
}

async function checkForUpdate(showCurrent: boolean): Promise<void> {
  if (state.running) {
    if (showCurrent) window.alert("Updates are deferred until the current job finishes.");
    return;
  }
  elements.checkUpdate.disabled = true;
  elements.checkUpdate.textContent = "Checking…";
  try {
    const update = await invoke<UpdateInfo | null>("check_update");
    if (!update) {
      if (showCurrent) window.alert("You are running the latest available version.");
      return;
    }
    const notes = update.notes ? `\n\n${update.notes}` : "";
    if (window.confirm(`Version ${update.version} is available.${notes}\n\nInstall and restart now?`)) {
      elements.checkUpdate.textContent = "Installing…";
      await invoke("install_update");
      await relaunch();
    }
  } catch (error) {
    if (showCurrent) window.alert(`Update check failed: ${String(error)}`);
  } finally {
    elements.checkUpdate.disabled = false;
    elements.checkUpdate.textContent = "Check for updates";
  }
}

function render(): void {
  elements.outputDir.value = state.outputDir;
  elements.browseFolder.disabled = state.running || state.scanning;
  elements.browseFiles.disabled = state.running || state.scanning;
  elements.browseOutput.disabled = state.running;
  elements.dropZone.classList.toggle("is-busy", state.scanning);
  elements.dropZone.querySelector("h2")!.textContent = state.scanning
    ? "Inspecting video channels…"
    : "Drop recordings or a folder";
  renderCandidates();
  renderSummary();
  renderProgress();
  renderResult();
}

function renderCandidates(): void {
  if (state.candidates.length === 0) {
    elements.candidateBody.innerHTML = `<tr class="empty-row"><td colspan="5">${escapeHtml(state.error || "No recordings scanned yet.")}</td></tr>`;
    return;
  }
  elements.candidateBody.innerHTML = state.candidates
    .map((candidate, index) => {
      const detected = candidate.recordingTime
        ? `${formatDate(candidate.recordingTime)}<small>${humanSource(candidate.timestampSource)}</small>`
        : "Unknown";
      const status = candidate.valid
        ? '<span class="status valid">Ready</span>'
        : `<span class="status invalid">Skipped</span><small class="reason">${escapeHtml(candidate.reason || "Invalid AVI")}</small>`;
      return `
        <tr class="${candidate.valid ? "" : "invalid-row"}">
          <td class="check-column"><input type="checkbox" data-id="${candidate.id}" ${candidate.included ? "checked" : ""} ${candidate.valid && !state.running ? "" : "disabled"} aria-label="Include ${escapeHtml(candidate.displayPath)}" /></td>
          <td><div class="recording-cell"><span class="sequence-number">${String(index + 1).padStart(2, "0")}</span><span title="${escapeHtml(candidate.path)}">${escapeHtml(candidate.displayPath)}</span></div></td>
          <td>${detected}</td>
          <td><span class="stream-count">${candidate.videoStreams}V · ${candidate.audioStreams}A</span>${candidate.durationSeconds ? `<small>${formatShortDuration(candidate.durationSeconds)}</small>` : ""}</td>
          <td>${status}</td>
        </tr>`;
    })
    .join("");
}

function renderSummary(): void {
  const selected = state.candidates.filter((candidate) => candidate.valid && candidate.included).length;
  const invalid = state.candidates.filter((candidate) => !candidate.valid).length;
  elements.selectionCount.textContent = state.pendingJob && selected === 0
    ? `${state.pendingJob.completedTasks}/${state.pendingJob.totalTasks} resumable tasks`
    : `${selected} selected${invalid ? ` · ${invalid} skipped` : ""}`;
  elements.startJob.disabled = state.running || state.scanning || (selected === 0 && !state.pendingJob) || !state.outputDir;
  elements.startJob.querySelector("span")!.textContent = state.running
    ? "Processing…"
    : state.pendingJob && selected === 0
      ? "Resume interrupted job"
      : `Process ${selected || "selected"} video${selected === 1 ? "" : "s"}`;
}

async function refreshPendingJob(): Promise<void> {
  if (!state.outputDir) {
    state.pendingJob = null;
    return;
  }
  try {
    state.pendingJob = await invoke<PendingJobInfo | null>("pending_job", {
      outputDir: state.outputDir,
    });
  } catch {
    state.pendingJob = null;
  }
}

function renderProgress(): void {
  const visible = state.running || Boolean(state.progress);
  elements.progressPanel.classList.toggle("hidden", !visible);
  if (!state.progress) return;
  const progress = state.progress;
  const percent = progress.totalTasks > 0
    ? Math.round((progress.completedTasks / progress.totalTasks) * 100)
    : 0;
  elements.progressStage.textContent = titleCase(progress.stage);
  elements.progressPercent.textContent = `${percent}%`;
  elements.progressBar.style.width = `${percent}%`;
  elements.progressFile.textContent = progress.currentFile || progress.message;
  elements.metricTasks.textContent = `${progress.completedTasks} / ${progress.totalTasks}`;
  elements.metricElapsed.textContent = formatClock(progress.elapsedSeconds);
  elements.metricEta.textContent = progress.etaSeconds == null ? "Calculating…" : formatClock(progress.etaSeconds);
  elements.jobLog.textContent = state.logs.join("\n");
}

function renderResult(): void {
  const visible = !state.running && (Boolean(state.result) || Boolean(state.error));
  elements.resultPanel.classList.toggle("hidden", !visible);
  elements.resultPanel.classList.toggle("error", Boolean(state.error));
  if (state.error) {
    elements.resultIcon.textContent = "!";
    elements.resultTitle.textContent = "Processing stopped";
    elements.resultMessage.textContent = state.error;
  } else if (state.result) {
    elements.resultIcon.textContent = "✓";
    elements.resultTitle.textContent = "Combined video created";
    elements.resultMessage.textContent = `${state.result.processedFiles} clips → ${state.result.outputPath}`;
  }
}

function inferredOutput(paths: string[]): string {
  if (paths.length === 1 && !/\.avi$/i.test(paths[0])) return paths[0];
  return commonParent(paths);
}

function commonParent(paths: string[]): string {
  const separator = paths.some((path) => path.includes("\\")) ? "\\" : "/";
  const parents = paths.map((path) => path.split(separator).slice(0, -1));
  const common: string[] = [];
  for (let index = 0; index < Math.min(...parents.map((parts) => parts.length)); index += 1) {
    const value = parents[0][index];
    if (parents.every((parts) => parts[index].toLocaleLowerCase() === value.toLocaleLowerCase())) common.push(value);
    else break;
  }
  if (separator === "/" && common.length === 0) return "/";
  return common.join(separator) || parents[0].join(separator);
}

function byId<T extends HTMLElement>(id: string): T {
  return document.getElementById(id) as T;
}

function formatDate(value: string): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "medium",
  }).format(new Date(value));
}

function humanSource(value?: string): string {
  return (value || "unknown").replaceAll("_", " ");
}

function formatClock(seconds: number): string {
  const whole = Math.max(0, Math.round(seconds));
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const remainder = whole % 60;
  return [hours, minutes, remainder].map((part) => String(part).padStart(2, "0")).join(":");
}

function formatShortDuration(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  return `${minutes}:${String(Math.round(seconds % 60)).padStart(2, "0")}`;
}

function titleCase(value: string): string {
  return value.replaceAll("_", " ").replace(/\b\w/g, (character) => character.toUpperCase());
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>'"]/g, (character) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    "'": "&#39;",
    '"': "&quot;",
  })[character]!);
}

render();
void initializeNativeEvents();
setTimeout(() => void checkForUpdate(false), 1500);
