use futures_util::StreamExt;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};

#[derive(serde::Serialize)]
pub struct FfmpegPaths {
  pub ffmpeg_path: String,
  pub ffprobe_path: String,
}

#[derive(serde::Serialize, Clone)]
struct ProgressEvent {
  file: String,        // "ffmpeg" | "ffprobe"
  downloaded: u64,
  total: Option<u64>,
  percent: Option<f64>,
}

fn bin_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
  Ok(app.path().app_data_dir()?.join("bin"))
}

async fn download(app: &AppHandle, url: &str, out: &Path, label: &str) -> Result<(), String> {
  let client = reqwest::Client::new();
  let res = client.get(url).send().await.map_err(|e| e.to_string())?;
  if !res.status().is_success() {
    return Err(format!("Failed to download {label}: HTTP {}", res.status()));
  }

  let total = res.content_length();
  std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
  let tmp = out.with_extension("part");
  let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;

  let mut downloaded = 0u64;
  let mut stream = res.bytes_stream();
  while let Some(chunk) = stream.next().await {
    let chunk = chunk.map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(&chunk).map_err(|e| e.to_string())?;
    downloaded += chunk.len() as u64;

    let percent = total.map(|t| downloaded as f64 / t as f64 * 100.0);

    let _ = app.emit(
      "ffmpeg_download_progress",
      ProgressEvent {
        file: label.to_string(),
        downloaded,
        total,
        percent,
      },
    );
  }

  drop(file);
  std::fs::rename(tmp, out).map_err(|e| e.to_string())?;

  // Ensure executable bit on Unix
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(out).map_err(|e| e.to_string())?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(out, perms).map_err(|e| e.to_string())?;
  }

  Ok(())
}

/// Downloads ffmpeg + ffprobe into app data dir if missing.
/// You should host the binaries as direct-download URLs (recommended: GitHub Release assets).
pub async fn ensure_ffmpeg(
  app: AppHandle,
  ffmpeg_url: String,
  ffprobe_url: String,
) -> Result<FfmpegPaths, String> {
  let dir = bin_dir(&app).map_err(|e| e.to_string())?;

  #[cfg(windows)]
  let (ffmpeg_name, ffprobe_name) = ("ffmpeg.exe", "ffprobe.exe");
  #[cfg(not(windows))]
  let (ffmpeg_name, ffprobe_name) = ("ffmpeg", "ffprobe");

  let ffmpeg_path = dir.join(ffmpeg_name);
  let ffprobe_path = dir.join(ffprobe_name);

  if !ffmpeg_path.exists() {
    download(&app, &ffmpeg_url, &ffmpeg_path, "ffmpeg").await?;
  }
  if !ffprobe_path.exists() {
    download(&app, &ffprobe_url, &ffprobe_path, "ffprobe").await?;
  }

  Ok(FfmpegPaths {
    ffmpeg_path: ffmpeg_path.to_string_lossy().to_string(),
    ffprobe_path: ffprobe_path.to_string_lossy().to_string(),
  })
}
