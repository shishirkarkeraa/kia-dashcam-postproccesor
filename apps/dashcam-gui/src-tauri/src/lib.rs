use kia_dashcam_core::{
    CancellationToken, DiscoveryRequest, JobEvent, JobPlan, JobResult, ToolPaths, VideoCandidate,
    discover_inputs, pending_job as find_pending_job, process_job, resolve_tool_paths,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::UpdaterExt;

struct ProcessingState {
    running: AtomicBool,
    cancellation: Mutex<Option<CancellationToken>>,
}

impl Default for ProcessingState {
    fn default() -> Self {
        Self {
            running: AtomicBool::new(false),
            cancellation: Mutex::new(None),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateInfo {
    version: String,
    notes: Option<String>,
    date: Option<String>,
}

#[tauri::command]
async fn scan_inputs(
    app: AppHandle,
    paths: Vec<PathBuf>,
    display_root: Option<PathBuf>,
) -> Result<Vec<VideoCandidate>, String> {
    let tools = gui_tools(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        Ok(discover_inputs(
            &DiscoveryRequest {
                paths,
                display_root,
            },
            &tools,
        ))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn start_job(
    app: AppHandle,
    state: State<'_, ProcessingState>,
    plan: JobPlan,
) -> Result<JobResult, String> {
    if state.running.swap(true, Ordering::SeqCst) {
        return Err("a processing job is already running".to_string());
    }
    let cancellation = CancellationToken::new();
    *state
        .cancellation
        .lock()
        .map_err(|_| "cancellation lock poisoned")? = Some(cancellation.clone());
    let tools = match gui_tools(&app) {
        Ok(tools) => tools,
        Err(error) => {
            state.running.store(false, Ordering::SeqCst);
            *state
                .cancellation
                .lock()
                .map_err(|_| "cancellation lock poisoned")? = None;
            return Err(error);
        }
    };
    let event_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        process_job(&plan, &tools, &cancellation, &move |event: JobEvent| {
            let _ = event_app.emit("job-event", event);
        })
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string());
    state.running.store(false, Ordering::SeqCst);
    *state
        .cancellation
        .lock()
        .map_err(|_| "cancellation lock poisoned")? = None;
    result
}

#[tauri::command]
fn cancel_job(state: State<'_, ProcessingState>) -> Result<(), String> {
    if let Some(token) = state
        .cancellation
        .lock()
        .map_err(|_| "cancellation lock poisoned")?
        .as_ref()
    {
        token.cancel();
    }
    Ok(())
}

#[tauri::command]
fn is_processing(state: State<'_, ProcessingState>) -> bool {
    state.running.load(Ordering::SeqCst)
}

#[tauri::command]
fn pending_job(output_dir: PathBuf) -> Result<Option<kia_dashcam_core::PendingJobInfo>, String> {
    find_pending_job(&output_dir).map_err(|error| error.to_string())
}

#[tauri::command]
async fn check_update(
    app: AppHandle,
    state: State<'_, ProcessingState>,
) -> Result<Option<UpdateInfo>, String> {
    if state.running.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let Some((endpoint, public_key)) = update_configuration() else {
        return Ok(None);
    };
    let update = app
        .updater_builder()
        .endpoints(vec![
            endpoint
                .parse()
                .map_err(|error| format!("invalid updater URL: {error}"))?,
        ])
        .map_err(|error| error.to_string())?
        .pubkey(public_key)
        .build()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;
    Ok(update.map(|update| UpdateInfo {
        version: update.version,
        notes: update.body,
        date: update.date.map(|date| date.to_string()),
    }))
}

#[tauri::command]
async fn install_update(app: AppHandle, state: State<'_, ProcessingState>) -> Result<(), String> {
    if state.running.load(Ordering::SeqCst) {
        return Err("updates are deferred until processing finishes".to_string());
    }
    let Some((endpoint, public_key)) = update_configuration() else {
        return Err("updater is not configured in this build".to_string());
    };
    let updater = app
        .updater_builder()
        .endpoints(vec![
            endpoint
                .parse()
                .map_err(|error| format!("invalid updater URL: {error}"))?,
        ])
        .map_err(|error| error.to_string())?
        .pubkey(public_key)
        .build()
        .map_err(|error| error.to_string())?;
    if let Some(update) = updater.check().await.map_err(|error| error.to_string())? {
        let event_app = app.clone();
        update
            .download_and_install(
                move |chunk, total| {
                    let _ = event_app.emit(
                        "update-progress",
                        serde_json::json!({ "chunk": chunk, "total": total }),
                    );
                },
                {
                    let event_app = app.clone();
                    move || {
                        let _ = event_app.emit("update-installed", ());
                    }
                },
            )
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn gui_tools(app: &AppHandle) -> Result<ToolPaths, String> {
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?;
    resolve_tool_paths(Some(&resource_dir)).map_err(|error| {
        format!("{error}. The internal media-tool bundle is incomplete; reinstall this application")
    })
}

fn update_configuration() -> Option<(String, &'static str)> {
    let repository = option_env!("KIA_DASHCAM_UPDATE_REPO")?;
    let public_key = option_env!("TAURI_SIGNING_PUBLIC_KEY")?;
    Some((
        format!("https://github.com/{repository}/releases/latest/download/latest.json"),
        public_key,
    ))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .manage(ProcessingState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init());
    let builder = if update_configuration().is_some() {
        builder.plugin(tauri_plugin_updater::Builder::new().build())
    } else {
        builder
    };
    builder
        .invoke_handler(tauri::generate_handler![
            scan_inputs,
            start_job,
            cancel_job,
            is_processing,
            pending_job,
            check_update,
            install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kia Dashcam Processor");
}
