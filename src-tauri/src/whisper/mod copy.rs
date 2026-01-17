use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager};

mod process;

static IS_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Clone)]
#[serde(tag = "kind")]
enum ProgressEvent {
  #[serde(rename = "stage")]
  Stage { stage: String, detail: Option<String> },

  #[serde(rename = "log")]
  Log { line: String },

  #[serde(rename = "done")]
  Done { outputPath: String },
}

fn emit(app: &AppHandle, evt: ProgressEvent) {
  let _ = app.emit("lyric_progress", evt);
}

struct RunningGuard;
impl Drop for RunningGuard {
  fn drop(&mut self) {
    IS_RUNNING.store(false, Ordering::SeqCst);
  }
}

fn whisper_supports_direct(path: &PathBuf) -> bool {
  match path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()) {
    Some(ext) if matches!(ext.as_str(), "mp3" | "wav" | "flac" | "ogg") => true,
    _ => false,
  }
}

pub async fn generate_lrc_next_to_audio(
  app: AppHandle,
  audio_path: &str,
  model: &str,
) -> Result<String, String> {
  // single-flight guard (prevents double-run from StrictMode / double-clicks)
  if IS_RUNNING.swap(true, Ordering::SeqCst) {
    return Err("Generation already running".into());
  }
  let _guard = RunningGuard;

  let audio_path = PathBuf::from(audio_path);
  if !audio_path.exists() {
    return Err("Audio file does not exist".into());
  }

  // Output path next to audio file
  let out_path = audio_path.with_extension("lrc");

  emit(
    &app,
    ProgressEvent::Stage {
      stage: "Preparing".into(),
      detail: Some("Locating resources".into()),
    },
  );

  let resources_dir = app
    .path()
    .resource_dir()
    .map_err(|e| format!("resource_dir error: {e}"))?;

  // In dev, resources may not be where we expect. Also check src-tauri/resources.
  let fallback_resources_dir = std::env::current_dir().ok().and_then(|cwd| {
    let candidates = vec![
      cwd.join("src-tauri").join("resources"),
      cwd.join("resources"),
      cwd.parent()
        .map(|p| p.join("src-tauri").join("resources"))
        .unwrap_or_else(|| cwd.join("__nope__")),
    ];

    for c in candidates {
      if c.exists() {
        return Some(c);
      }
    }
    None
  });

  let platform = if cfg!(target_os = "macos") {
    "macos"
  } else if cfg!(target_os = "windows") {
    "windows"
  } else {
    return Err("Unsupported OS".into());
  };

  let bin_dir = resources_dir.join("bin").join(platform);
  let ffmpeg =
    process::pick_executable_with_fallback(&bin_dir, fallback_resources_dir.as_ref(), platform, "ffmpeg")?;
  let whisper =
    process::pick_executable_with_fallback(&bin_dir, fallback_resources_dir.as_ref(), platform, "whisper")?;

  // Temp workspace (unique per run)
  let run_id = format!(
    "{}",
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map_err(|e| format!("time error: {e}"))?
      .as_millis()
  );

  let tmp_dir = std::env::temp_dir().join("lyrictime").join(run_id);
  std::fs::create_dir_all(&tmp_dir).map_err(|e| format!("temp dir create failed: {e}"))?;

  // Choose input for whisper
  let direct = whisper_supports_direct(&audio_path);
  let wav_path = tmp_dir.join("input.wav");

  let whisper_input = if direct {
    emit(
      &app,
      ProgressEvent::Stage {
        stage: "Preparing".into(),
        detail: Some("Input format supported by whisper (skipping ffmpeg)".into()),
      },
    );
    audio_path.clone()
  } else {
    emit(
      &app,
      ProgressEvent::Stage {
        stage: "Converting".into(),
        detail: Some("Unsupported format → ffmpeg → 16k mono WAV".into()),
      },
    );
    process::run_ffmpeg_to_wav(&app, &ffmpeg, &audio_path, &wav_path)?;
    wav_path.clone()
  };

  // HYBRID MODE:
  // - Always run small (best coverage for lyrics).
  // - Then run medium (if installed) and merge: use medium where it has confident lines,
  //   fallback to small for gaps.
  let use_hybrid = model.eq_ignore_ascii_case("hybrid");

  if use_hybrid {
    emit(
      &app,
      ProgressEvent::Stage {
        stage: "Transcribing".into(),
        detail: Some("Hybrid: small pass".into()),
      },
    );

    let small_model_path = process::resolve_model_path_with_fallback(
      &app,
      &resources_dir,
      fallback_resources_dir.as_ref(),
      "small",
    )?;

    let out_small_prefix = tmp_dir.join("out_small");
    process::run_whisper_lrc(&app, &whisper, &small_model_path, &whisper_input, &out_small_prefix)?;

    let small_lrc_path = out_small_prefix.with_extension("lrc");
    if !small_lrc_path.exists() {
      return Err("Whisper (small) did not produce LRC".into());
    }

    let raw_small = std::fs::read_to_string(&small_lrc_path)
      .map_err(|e| format!("Failed reading small LRC: {e}"))?;
    let small_clean = clean_lrc(&raw_small);

    emit(
      &app,
      ProgressEvent::Stage {
        stage: "Transcribing".into(),
        detail: Some("Hybrid: medium pass".into()),
      },
    );

    let medium_model_path = process::resolve_model_path_with_fallback(
      &app,
      &resources_dir,
      fallback_resources_dir.as_ref(),
      "medium",
    )?;

    let out_medium_prefix = tmp_dir.join("out_medium");
    process::run_whisper_lrc(
      &app,
      &whisper,
      &medium_model_path,
      &whisper_input,
      &out_medium_prefix,
    )?;

    let medium_lrc_path = out_medium_prefix.with_extension("lrc");
    if !medium_lrc_path.exists() {
      return Err("Whisper (medium) did not produce LRC".into());
    }

    let raw_medium = std::fs::read_to_string(&medium_lrc_path)
      .map_err(|e| format!("Failed reading medium LRC: {e}"))?;
    let medium_clean = clean_lrc(&raw_medium);

    emit(
      &app,
      ProgressEvent::Stage {
        stage: "Merging".into(),
        detail: Some("Hybrid merge: prefer medium, fill gaps with small".into()),
      },
    );

    let merged = merge_lrc_prefer_medium(&small_clean, &medium_clean);

    emit(
      &app,
      ProgressEvent::Stage {
        stage: "Writing".into(),
        detail: Some("Writing merged .lrc next to audio".into()),
      },
    );

    std::fs::write(&out_path, merged).map_err(|e| format!("Failed writing merged LRC: {e}"))?;

    emit(
      &app,
      ProgressEvent::Done {
        outputPath: out_path.display().to_string(),
      },
    );

    return Ok(out_path.display().to_string());
  }

  // NON-HYBRID: single pass using requested model ("small" or "medium")
  emit(
    &app,
    ProgressEvent::Stage {
      stage: "Transcribing".into(),
      detail: Some("Running whisper".into()),
    },
  );

  let model_path =
    process::resolve_model_path_with_fallback(&app, &resources_dir, fallback_resources_dir.as_ref(), model)?;

  let out_prefix = tmp_dir.join("out");
  process::run_whisper_lrc(&app, &whisper, &model_path, &whisper_input, &out_prefix)?;

  emit(
    &app,
    ProgressEvent::Stage {
      stage: "Writing".into(),
      detail: Some("Copying .lrc next to audio".into()),
    },
  );

  let produced_lrc = out_prefix.with_extension("lrc");
  if !produced_lrc.exists() {
    return Err(format!(
      "Whisper did not produce an .lrc file at {}",
      produced_lrc.display()
    ));
  }

  let raw_lrc = std::fs::read_to_string(&produced_lrc)
    .map_err(|e| format!("Failed reading produced LRC: {e}"))?;

  let cleaned = clean_lrc(&raw_lrc);

  std::fs::write(&out_path, cleaned)
    .map_err(|e| format!("Failed writing cleaned LRC: {e}"))?;

  emit(
    &app,
    ProgressEvent::Done {
      outputPath: out_path.display().to_string(),
    },
  );

  Ok(out_path.display().to_string())
}

fn parse_lrc_lines(input: &str) -> Vec<(String, String)> {
  let mut out = Vec::new();

  for line in input.lines() {
    let l = line.trim();
    if !l.starts_with('[') {
      continue;
    }
    if let Some(end) = l.find(']') {
      let ts = l[..=end].to_string();
      let text = l[end + 1..].trim().to_string();
      if !text.is_empty() {
        out.push((ts, text));
      }
    }
  }

  out
}

fn merge_lrc_prefer_medium(small: &str, medium: &str) -> String {
  let small_lines = parse_lrc_lines(small);
  let medium_lines = parse_lrc_lines(medium);

  let mut medium_map: HashMap<String, String> = HashMap::new();
  let mut medium_order: Vec<String> = Vec::new();
  for (ts, text) in medium_lines {
    // keep first occurrence (stable)
    if !medium_map.contains_key(&ts) {
      medium_order.push(ts.clone());
      medium_map.insert(ts, text);
    }
  }

  let mut seen_ts: HashSet<String> = HashSet::new();
  let mut merged = String::new();

  // baseline order: small (more complete)
  for (ts, small_text) in small_lines {
    let text = medium_map.get(&ts).cloned().unwrap_or(small_text);
    merged.push_str(&ts);
    merged.push(' ');
    merged.push_str(text.trim());
    merged.push('\n');
    seen_ts.insert(ts);
  }

  // append any medium-only timestamps that small never emitted
  for ts in medium_order {
    if seen_ts.contains(&ts) {
      continue;
    }
    if let Some(text) = medium_map.get(&ts) {
      merged.push_str(&ts);
      merged.push(' ');
      merged.push_str(text.trim());
      merged.push('\n');
    }
  }

  merged
}

fn clean_lrc(input: &str) -> String {
  let mut out = String::new();

  for line in input.lines() {
    let l = line.trim();
    if l.is_empty() {
      continue;
    }

    // Drop metadata tags like [by:whisper.cpp], [ar:...], etc.
    if l.starts_with('[') {
      if let Some(end) = l.find(']') {
        let inside = &l[1..end];
        // If it's a tag (contains ':' and doesn't start with a digit), drop it.
        if inside.contains(':')
          && inside
            .chars()
            .next()
            .map(|c| !c.is_ascii_digit())
            .unwrap_or(false)
        {
          continue;
        }
      }
    }

    // Timestamp line: [mm:ss.xx]text
    if l.starts_with('[') {
      if let Some(end) = l.find(']') {
        let (ts, rest) = l.split_at(end + 1);
        let mut text = rest.trim().replace('♪', "").trim().to_string();

        // Drop music cue lines like "(upbeat music)"
        if text.starts_with('(') && text.ends_with(')') {
          continue;
        }

        if text.is_empty() {
          continue;
        }

        while text.contains("  ") {
          text = text.replace("  ", " ");
        }

        out.push_str(ts);
        out.push(' ');
        out.push_str(text.trim());
        out.push('\n');
        continue;
      }
    }

    // Otherwise keep non-timestamp lines (rare), but also strip ♪
    let cleaned = l.replace('♪', "").trim().to_string();
    if !cleaned.is_empty() {
      out.push_str(&cleaned);
      out.push('\n');
    }
  }

  out
}
