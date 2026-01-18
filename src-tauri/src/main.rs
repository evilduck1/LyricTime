#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod whisper;
mod model_downloader;
mod ffmpeg_downloader;
mod download;

#[tauri::command]
async fn generate_lrc_next_to_audio(
  app: tauri::AppHandle,
  audio_path: String,
  model: String,
) -> Result<String, String> {
  whisper::generate_lrc_next_to_audio(app, &audio_path, &model).await
}

#[tauri::command]
async fn ensure_models_downloaded(
  app: tauri::AppHandle,
) -> Result<model_downloader::ModelPaths, String> {
  // NOTE: GitHub Releases are flat files (no folders). Upload these as assets
  // under tag `models`: ggml-small.bin and ggml-medium.bin
  let small = "https://github.com/evilduck1/LyricTime/releases/download/models/ggml-small.bin".to_string();
  let medium = "https://github.com/evilduck1/LyricTime/releases/download/models/ggml-medium.bin".to_string();
  model_downloader::ensure_models(app, small, medium).await
}

#[tauri::command]
async fn ensure_ffmpeg_downloaded(
  app: tauri::AppHandle,
) -> Result<ffmpeg_downloader::FfmpegPaths, String> {
  // NOTE: GitHub Releases are flat files (no folders). Upload these 4 files as assets
  // under tag `deps`: ffmpeg.exe, ffprobe.exe, ffmpeg, ffprobe
  #[cfg(windows)]
  let (ffmpeg_url, ffprobe_url) = (
    "https://github.com/evilduck1/LyricTime/releases/download/deps/ffmpeg.exe".to_string(),
    "https://github.com/evilduck1/LyricTime/releases/download/deps/ffprobe.exe".to_string(),
  );

  #[cfg(not(windows))]
  let (ffmpeg_url, ffprobe_url) = (
    "https://github.com/evilduck1/LyricTime/releases/download/deps/ffmpeg".to_string(),
    "https://github.com/evilduck1/LyricTime/releases/download/deps/ffprobe".to_string(),
  );

  ffmpeg_downloader::ensure_ffmpeg(app, ffmpeg_url, ffprobe_url).await
}

fn main() {
  tauri::Builder::default()
    .plugin(tauri_plugin_dialog::init())
    .invoke_handler(tauri::generate_handler![
      generate_lrc_next_to_audio,
      ensure_models_downloaded,
      ensure_ffmpeg_downloaded
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
