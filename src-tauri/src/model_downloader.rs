use futures_util::StreamExt;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};

#[derive(serde::Serialize)]
pub struct ModelPaths {
  pub small_path: String,
  pub medium_path: String,
}

#[derive(serde::Serialize, Clone)]
struct ProgressEvent {
  model: String,
  downloaded: u64,
  total: Option<u64>,
  percent: Option<f64>,
}

fn models_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
  Ok(app.path().app_data_dir()?.join("models"))
}

async fn download(app: &AppHandle, url: &str, out: &Path, name: &str) -> Result<(), String> {
  let client = reqwest::Client::new();
  let res = client.get(url).send().await.map_err(|e| e.to_string())?;
  if !res.status().is_success() {
    return Err(format!("Failed to download {name}: HTTP {}", res.status()));
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

    // Requires `use tauri::Emitter;`
    let _ = app.emit(
      "model_download_progress",
      ProgressEvent {
        model: name.to_string(),
        downloaded,
        total,
        percent,
      },
    );
  }

  // Close file handle before rename on Windows
  drop(file);

  std::fs::rename(tmp, out).map_err(|e| e.to_string())?;
  Ok(())
}

pub async fn ensure_models(
  app: AppHandle,
  small_url: String,
  medium_url: String,
) -> Result<ModelPaths, String> {
  let dir = models_dir(&app).map_err(|e| e.to_string())?;
  let small = dir.join("ggml-small.bin");
  let medium = dir.join("ggml-medium.bin");

  if !small.exists() {
    download(&app, &small_url, &small, "small").await?;
  }
  if !medium.exists() {
    download(&app, &medium_url, &medium, "medium").await?;
  }

  Ok(ModelPaths {
    small_path: small.to_string_lossy().to_string(),
    medium_path: medium.to_string_lossy().to_string(),
  })
}
