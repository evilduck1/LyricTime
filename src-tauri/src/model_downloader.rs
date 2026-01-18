use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

use crate::download;


#[derive(serde::Serialize)]
pub struct ModelPaths {
  pub small_path: String,
  pub medium_path: String,
}

fn models_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
  Ok(app.path().app_data_dir()?.join("models"))
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
    download::download_with_progress(&app, "models", &small_url, &small, "ggml-small.bin").await?;
  }
  if !medium.exists() {
    download::download_with_progress(&app, "models", &medium_url, &medium, "ggml-medium.bin").await?;
  }

  Ok(ModelPaths {
    small_path: small.to_string_lossy().to_string(),
    medium_path: medium.to_string_lossy().to_string(),
  })
}
