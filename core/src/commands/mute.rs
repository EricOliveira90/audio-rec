use anyhow::Context;
use serde_json::json;
use std::path::PathBuf;

use crate::config::RecorderConfig;
use crate::error::Result;
use crate::status::RecordingStatus;

pub async fn execute(session_id: Option<String>, config: RecorderConfig) -> Result<()> {
    config.ensure_directories()?;

    let target_session = if let Some(id) = session_id {
        id
    } else {
        find_active_session(&config.status_dir)?
    };

    let signal_path = config.signals_dir.join(format!("{}.mute", target_session));
    std::fs::write(&signal_path, "")
        .with_context(|| format!("Failed to create mute signal file: {:?}", signal_path))?;

    let result = json!({
        "status": "success",
        "data": {
            "session_id": target_session,
            "message": "Mute toggle signal sent successfully"
        }
    });

    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

fn find_active_session(status_dir: &PathBuf) -> Result<String> {
    use std::fs;

    let entries = fs::read_dir(status_dir).context("Failed to read status directory")?;

    let mut active_sessions = Vec::new();

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(status) = serde_json::from_str::<RecordingStatus>(&content) {
                if status.status == "recording" {
                    active_sessions.push((status.session_id, path));
                }
            }
        }
    }

    if active_sessions.is_empty() {
        return Err(anyhow::anyhow!("No active recording sessions found").into());
    }

    active_sessions.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(active_sessions[0].0.clone())
}
