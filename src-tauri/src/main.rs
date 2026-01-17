#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod whisper;

#[tauri::command]
async fn generate_lrc_next_to_audio(
  app: tauri::AppHandle,
  audio_path: String,
  model: String,
) -> Result<String, String> {
  whisper::generate_lrc_next_to_audio(app, &audio_path, &model).await
}

fn main() {
  tauri::Builder::default()
    .plugin(tauri_plugin_dialog::init())
    .invoke_handler(tauri::generate_handler![generate_lrc_next_to_audio])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
