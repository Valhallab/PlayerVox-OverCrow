use overcrow_control::{
    ControlCommandService, ControlLogSnapshot, ControlSnapshot, ControlSupportReceipt,
    ControlSupportReport, SupportReportClient, SupportReportError,
    prepare_support_report as build_support_report,
};
use std::sync::Mutex;
use tauri::{AppHandle, State};

pub type CommandState = ControlCommandService;

#[derive(Default)]
pub struct SupportReportState {
    inner: Mutex<PreparedSupportReport>,
}

#[derive(Default)]
struct PreparedSupportReport {
    report: Option<ControlSupportReport>,
    submission_in_progress: bool,
}

impl SupportReportState {
    fn store(&self, report: ControlSupportReport) -> Result<(), String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| support_error(SupportReportError::ReportUnavailable))?;
        if state.submission_in_progress {
            return Err(support_error(SupportReportError::SubmissionBusy));
        }
        state.report = Some(report);
        Ok(())
    }

    fn begin_submission(&self, report_id: &str) -> Result<ControlSupportReport, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| support_error(SupportReportError::ReportUnavailable))?;
        if state.submission_in_progress {
            return Err(support_error(SupportReportError::SubmissionBusy));
        }
        let report = state
            .report
            .as_ref()
            .filter(|report| report.report_id == report_id)
            .cloned()
            .ok_or_else(|| support_error(SupportReportError::ReportUnavailable))?;
        state.submission_in_progress = true;
        Ok(report)
    }

    fn finish_submission(&self, report_id: &str, succeeded: bool) -> Result<(), String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| support_error(SupportReportError::ReportUnavailable))?;
        state.submission_in_progress = false;
        if succeeded
            && state
                .report
                .as_ref()
                .is_some_and(|report| report.report_id == report_id)
        {
            state.report = None;
        }
        Ok(())
    }
}

#[tauri::command]
pub fn get_control_state(
    app: AppHandle,
    state: State<'_, CommandState>,
) -> Result<ControlSnapshot, String> {
    sync_tray(&app, state.get_control_state())
}

#[tauri::command]
pub fn refresh_games(
    app: AppHandle,
    state: State<'_, CommandState>,
) -> Result<ControlSnapshot, String> {
    sync_tray(&app, state.refresh_games())
}

#[tauri::command]
pub fn set_game_selected(
    app: AppHandle,
    state: State<'_, CommandState>,
    app_id: u32,
    selected: bool,
) -> Result<ControlSnapshot, String> {
    sync_tray(&app, state.set_game_selected(app_id, selected))
}

#[tauri::command]
pub fn remove_manual_game(
    app: AppHandle,
    state: State<'_, CommandState>,
    id: String,
) -> Result<ControlSnapshot, String> {
    sync_tray(&app, state.remove_manual_game(&id))
}

#[tauri::command]
pub fn pick_manual_game(
    app: AppHandle,
    state: State<'_, CommandState>,
) -> Result<ControlSnapshot, String> {
    sync_tray(&app, state.pick_manual_game())
}

#[tauri::command]
pub fn set_overcrow_enabled(
    app: AppHandle,
    state: State<'_, CommandState>,
    enabled: bool,
) -> Result<ControlSnapshot, String> {
    let result = sync_tray(&app, state.set_overcrow_enabled(enabled));
    if result
        .as_ref()
        .is_ok_and(|snapshot| snapshot.operations.lifecycle)
    {
        crate::tray::ensure_lifecycle_monitor(&app);
    }
    result
}

#[tauri::command]
pub fn get_recent_logs(state: State<'_, CommandState>) -> Result<ControlLogSnapshot, String> {
    state.get_recent_logs()
}

#[tauri::command]
pub fn prepare_support_report(
    state: State<'_, CommandState>,
    support_state: State<'_, SupportReportState>,
    description: String,
    include_logs: bool,
) -> Result<ControlSupportReport, String> {
    let snapshot = state.get_control_state()?;
    let logs = include_logs.then(|| state.get_recent_logs()).transpose()?;
    let report =
        build_support_report(&snapshot, logs.as_ref(), &description).map_err(support_error)?;
    support_state.store(report.clone())?;
    Ok(report)
}

#[tauri::command]
pub async fn submit_support_report(
    support_state: State<'_, SupportReportState>,
    report_id: String,
) -> Result<ControlSupportReceipt, String> {
    let report = support_state.begin_submission(&report_id)?;
    let submission = tauri::async_runtime::spawn_blocking(move || {
        SupportReportClient::default()
            .submit(&report)
            .map_err(support_error)
    })
    .await
    .map_err(|_| "support_worker_failed".to_owned())
    .and_then(|result| result);
    support_state.finish_submission(&report_id, submission.is_ok())?;
    submission
}

fn support_error(error: overcrow_control::SupportReportError) -> String {
    error.code().to_owned()
}

fn sync_tray(
    app: &AppHandle,
    result: Result<ControlSnapshot, String>,
) -> Result<ControlSnapshot, String> {
    if let Ok(snapshot) = &result {
        crate::tray::sync_snapshot(app, snapshot);
    }
    result
}

#[cfg(test)]
mod tests {
    use overcrow_control::ControlSupportReport;

    use super::SupportReportState;

    fn report(id: &str) -> ControlSupportReport {
        ControlSupportReport {
            schema_version: 1,
            report_id: id.to_owned(),
            created_at: "2026-07-25T10:00:00Z".to_owned(),
            content: "report".to_owned(),
            logs_included: false,
        }
    }

    #[test]
    fn prepared_report_state_allows_only_the_exact_latest_report() {
        let state = SupportReportState::default();
        state.store(report("oc-first")).expect("store first");
        assert!(state.begin_submission("oc-other").is_err());

        let prepared = state.begin_submission("oc-first").expect("begin");
        assert_eq!(prepared.report_id, "oc-first");
        assert!(state.begin_submission("oc-first").is_err());
        assert!(state.store(report("oc-second")).is_err());

        state
            .finish_submission("oc-first", false)
            .expect("failed submission");
        assert!(state.begin_submission("oc-first").is_ok());
    }

    #[test]
    fn successful_submission_forgets_the_native_copy() {
        let state = SupportReportState::default();
        state.store(report("oc-first")).expect("store report");
        state.begin_submission("oc-first").expect("begin");
        state
            .finish_submission("oc-first", true)
            .expect("successful submission");

        assert!(state.begin_submission("oc-first").is_err());
    }
}
