use super::{emit, ProgressEvent};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::{AppHandle, Manager};

fn model_candidates(model: &str) -> Result<Vec<&'static str>, String> {
  match model {
    "small" => Ok(vec![
      "ggml-small.bin",
      "ggml-model-whisper-small.bin",
      "ggml-model-whisper-small-q5_1.bin",
      "ggml-model-whisper-small-q8_0.bin",
      "ggml-small-q8_0.bin",
      "ggml-small-q5_1.bin",
    ]),
    "medium" => Ok(vec![
      "ggml-medium.bin",
      "ggml-model-whisper-medium.bin",
      "ggml-model-whisper-medium-q5_0.bin",
      "ggml-model-whisper-medium-q8_0.bin",
      "ggml-medium-q8_0.bin",
      "ggml-medium-q5_0.bin",
    ]),
    _ => Err(format!("Unknown model: {model}")),
  }
}

fn search_dir_for_model(dir: &Path, candidates: &[&str]) -> Option<PathBuf> {
  let entries: Vec<fs::DirEntry> = fs::read_dir(dir).ok()?.filter_map(|r| r.ok()).collect();

  // 1) exact filename match (preferred order)
  for &name_wanted in candidates {
    for e in &entries {
      if e.file_name() == name_wanted {
        return Some(e.path());
      }
    }
  }

  // 2) prefix match: any file that starts with the candidate stem and ends with .bin
  for &name_wanted in candidates {
    let stem = name_wanted.trim_end_matches(".bin");
    for e in &entries {
      let name: String = e.file_name().to_string_lossy().into_owned();
      if name.starts_with(stem) && name.ends_with(".bin") {
        return Some(e.path());
      }
    }
  }

  None
}

fn exe_name(base: &str) -> String {
  if cfg!(target_os = "windows") {
    format!("{base}.exe")
  } else {
    base.to_string()
  }
}

pub fn pick_executable_with_fallback(
  bin_dir: &Path,
  fallback: Option<&PathBuf>,
  platform: &str,
  base: &str,
) -> Result<PathBuf, String> {
  let primary = bin_dir.join(exe_name(base));
  if primary.exists() {
    return Ok(primary);
  }

  if let Some(fallback) = fallback {
    let alt = fallback.join("bin").join(platform).join(exe_name(base));
    if alt.exists() {
      return Ok(alt);
    }
  }

  Err(format!("Executable not found: {base}"))
}

pub fn pick_executable_multi(
  app_bin_dir: &Path,
  resources_bin_dir: &Path,
  fallback: Option<&PathBuf>,
  platform: &str,
  base: &str,
) -> Result<PathBuf, String> {
  // 1) Downloaded binary in app data dir (preferred)
  let app_primary = app_bin_dir.join(exe_name(base));
  if app_primary.exists() {
    return Ok(app_primary);
  }

  // 2) Bundled resources/bin/<platform>
  let res_primary = resources_bin_dir.join(exe_name(base));
  if res_primary.exists() {
    return Ok(res_primary);
  }

  // 3) Dev fallback resources/bin/<platform>
  if let Some(fallback) = fallback {
    let alt = fallback.join("bin").join(platform).join(exe_name(base));
    if alt.exists() {
      return Ok(alt);
    }
  }

  Err(format!("Executable not found: {base}"))
}

pub fn resolve_model_path_with_fallback(
  app: &AppHandle,
  resources_dir: &Path,
  fallback: Option<&PathBuf>,
  model: &str,
) -> Result<PathBuf, String> {
  let candidates = model_candidates(model)?;

  let mut dirs: Vec<PathBuf> = Vec::new();

  // Downloaded models (app data)
  if let Ok(app_data) = app.path().app_data_dir() {
    dirs.push(app_data.join("models"));
  }

  // Bundled models
  dirs.push(resources_dir.join("models"));

  // Dev fallback resources/models
  if let Some(fb) = fallback {
    dirs.push(fb.join("models"));
  }

  // Common dev location: src-tauri/target/debug/models
  if let Ok(cwd) = std::env::current_dir() {
    dirs.push(cwd.join("target").join("debug").join("models"));
  }

  for dir in &dirs {
    if !dir.exists() {
      continue;
    }
    if let Some(found) = search_dir_for_model(dir, &candidates) {
      return Ok(found);
    }
  }

  Err(format!(
    "Model '{model}' not installed. Expected one of: {}",
    candidates.join(", ")
  ))
}

fn spawn_and_stream(app: &AppHandle, mut cmd: Command, label: &str) -> Result<(), String> {
  emit(
    app,
    ProgressEvent::Log {
      line: format!("Running {label}…"),
    },
  );

  let mut child = cmd
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|e| format!("Failed spawning {label}: {e}"))?;

  if let Some(stderr) = child.stderr.take() {
    let app2 = app.clone();
    std::thread::spawn(move || {
      use std::io::{BufRead, BufReader};
      let reader = BufReader::new(stderr);
      for line in reader.lines().flatten() {
        emit(&app2, ProgressEvent::Log { line });
      }
    });
  }

  let status = child
    .wait()
    .map_err(|e| format!("Failed waiting for {label}: {e}"))?;

  if !status.success() {
    return Err(format!("{label} failed with status: {status}"));
  }

  Ok(())
}

pub fn run_ffmpeg_to_wav(
  app: &AppHandle,
  ffmpeg: &Path,
  input: &Path,
  output_wav: &Path,
) -> Result<(), String> {
  let mut cmd = Command::new(ffmpeg);
  cmd.args([
    "-y",
    "-i",
    input.to_str().ok_or("Invalid input path")?,
    "-ac",
    "1",
    "-ar",
    "16000",
    output_wav.to_str().ok_or("Invalid output path")?,
  ]);

  spawn_and_stream(app, cmd, "ffmpeg")
}

pub fn run_whisper_lrc(
  app: &AppHandle,
  whisper: &Path,
  model: &Path,
  input_audio: &Path,
  out_prefix: &Path,
) -> Result<(), String> {
  let mut cmd = Command::new(whisper);
  cmd.args([
    "-m",
    model.to_str().ok_or("Invalid model path")?,
    "-olrc",
    "-of",
    out_prefix.to_str().ok_or("Invalid output prefix")?,
    input_audio.to_str().ok_or("Invalid input audio path")?,
  ]);

  spawn_and_stream(app, cmd, "whisper")
}
