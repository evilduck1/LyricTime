use futures_util::StreamExt;
use serde::Serialize;
use std::{
  fs,
  io::Write,
  path::Path,
  time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};

/// Unified download progress event used by deps + models.
/// Frontend listens to: `download://progress`
#[derive(Serialize, Clone, Debug)]
pub struct DownloadProgressEvent {
  pub group: String,           // "deps" | "models"
  pub file: String,            // filename shown to user
  pub downloaded_bytes: u64,
  pub total_bytes: Option<u64>,
  pub status: String,          // "downloading" | "done" | "error"
  pub error: Option<String>,
}

fn emit(app: &AppHandle, evt: DownloadProgressEvent) {
  let _ = app.emit("download://progress", evt);
}

/// Download a file with streamed progress.
///
/// - Writes to `<dest>.part` and renames on success
/// - Emits throttled progress events (default ~150ms)
/// - Caller can set executable bit separately if needed
pub async fn download_with_progress(
  app: &AppHandle,
  group: &str,
  url: &str,
  dest: &Path,
  display_name: &str,
) -> Result<(), String> {
  if let Some(parent) = dest.parent() {
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  }

  let client = reqwest::Client::new();
  let res = client.get(url).send().await.map_err(|e| e.to_string())?;
  if !res.status().is_success() {
    let msg = format!("Failed to download {display_name}: HTTP {}", res.status());
    emit(
      app,
      DownloadProgressEvent {
        group: group.to_string(),
        file: display_name.to_string(),
        downloaded_bytes: 0,
        total_bytes: None,
        status: "error".into(),
        error: Some(msg.clone()),
      },
    );
    return Err(msg);
  }

  let total = res.content_length();
  let tmp = dest.with_extension("part");
  // Clear old partial if any
  let _ = fs::remove_file(&tmp);

  let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;

  let mut downloaded: u64 = 0;
  let mut stream = res.bytes_stream();

  let mut last_emit = Instant::now();
  let min_interval = Duration::from_millis(150);

  emit(
    app,
    DownloadProgressEvent {
      group: group.to_string(),
      file: display_name.to_string(),
      downloaded_bytes: 0,
      total_bytes: total,
      status: "downloading".into(),
      error: None,
    },
  );

  while let Some(chunk) = stream.next().await {
    let chunk = chunk.map_err(|e| e.to_string())?;
    f.write_all(&chunk).map_err(|e| e.to_string())?;
    downloaded += chunk.len() as u64;

    if last_emit.elapsed() >= min_interval {
      emit(
        app,
        DownloadProgressEvent {
          group: group.to_string(),
          file: display_name.to_string(),
          downloaded_bytes: downloaded,
          total_bytes: total,
          status: "downloading".into(),
          error: None,
        },
      );
      last_emit = Instant::now();
    }
  }

  // Close file before rename (important on Windows)
  drop(f);

  fs::rename(&tmp, dest).map_err(|e| e.to_string())?;

  emit(
    app,
    DownloadProgressEvent {
      group: group.to_string(),
      file: display_name.to_string(),
      downloaded_bytes: downloaded,
      total_bytes: total,
      status: "done".into(),
      error: None,
    },
  );

  Ok(())
}
