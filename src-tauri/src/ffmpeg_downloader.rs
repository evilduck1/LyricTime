use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};

use crate::download;


#[derive(serde::Serialize)]
pub struct FfmpegPaths {
  pub ffmpeg_path: String,
  pub ffprobe_path: String,
}

fn bin_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
  Ok(app.path().app_data_dir()?.join("bin"))
}




fn ensure_executable(path: &Path) -> Result<(), String> {
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).map_err(|e| e.to_string())?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(|e| e.to_string())?;
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
    download::download_with_progress(&app, "deps", &ffmpeg_url, &ffmpeg_path, ffmpeg_name).await?;
    ensure_executable(&ffmpeg_path)?;
  }
  if !ffprobe_path.exists() {
    download::download_with_progress(&app, "deps", &ffprobe_url, &ffprobe_path, ffprobe_name).await?;
    ensure_executable(&ffprobe_path)?;
  }

  Ok(FfmpegPaths {
    ffmpeg_path: ffmpeg_path.to_string_lossy().to_string(),
    ffprobe_path: ffprobe_path.to_string_lossy().to_string(),
  })
}
