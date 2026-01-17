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

  // HYBRID+ (invisible):
  // - When model == "hybrid", run small + (optional) medium.
  // - Merge is chant-aware and timestamps are normalized.
  let use_hybrid = model.eq_ignore_ascii_case("hybrid");

  if use_hybrid {
    emit(
      &app,
      ProgressEvent::Stage {
        stage: "Transcribing".into(),
        detail: Some("Hybrid+: small pass".into()),
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
        detail: Some("Hybrid+: medium pass".into()),
      },
    );

    // Medium is optional. If it's not installed, silently fall back to small-only.
    let medium_model_path = match process::resolve_model_path_with_fallback(
      &app,
      &resources_dir,
      fallback_resources_dir.as_ref(),
      "medium",
    ) {
      Ok(p) => Some(p),
      Err(_) => None,
    };

    let merged = if let Some(medium_model_path) = medium_model_path {
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
        normalize_lrc_timestamps(&small_clean, 250)
      } else {
        let raw_medium = std::fs::read_to_string(&medium_lrc_path)
          .map_err(|e| format!("Failed reading medium LRC: {e}"))?;
        let medium_clean = clean_lrc(&raw_medium);

        emit(
          &app,
          ProgressEvent::Stage {
            stage: "Merging".into(),
            detail: Some("Hybrid+: chant-aware merge + timestamp normalization".into()),
          },
        );

        merge_hybrid_plus(&small_clean, &medium_clean)
      }
    } else {
      normalize_lrc_timestamps(&small_clean, 250)
    };

    emit(
      &app,
      ProgressEvent::Stage {
        stage: "Writing".into(),
        detail: Some("Writing .lrc next to audio".into()),
      },
    );

    std::fs::write(&out_path, merged).map_err(|e| format!("Failed writing LRC: {e}"))?;

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

/* -------------------- Hybrid+ merge helpers -------------------- */

#[derive(Clone, Debug)]
struct LrcLine {
  ms: i64,
  text: String,
}

fn normalize_text_key(s: &str) -> String {
  let t = s.trim().to_ascii_lowercase();
  let mut out = String::with_capacity(t.len());
  let mut last_space = false;

  for ch in t.chars() {
    let is_space = ch.is_whitespace();
    if is_space {
      if !last_space {
        out.push(' ');
      }
      last_space = true;
      continue;
    }

    if ch.is_ascii_alphanumeric() || matches!(ch, '\'' | '-' | ',' | '.' | '?' | '!') {
      out.push(ch);
      last_space = false;
    }
  }

  out.trim().to_string()
}

fn word_count(s: &str) -> usize {
  s.split_whitespace().filter(|w| !w.is_empty()).count()
}

fn parse_ts_to_ms(ts: &str) -> Option<i64> {
  // expects like [mm:ss.xx] or [mm:ss.xxx]
  let t = ts.trim().trim_start_matches('[').trim_end_matches(']');
  let mut parts = t.split(':');
  let mm = parts.next()?.parse::<i64>().ok()?;
  let rest = parts.next()?;
  let mut parts2 = rest.split('.');
  let ss = parts2.next()?.parse::<i64>().ok()?;
  let frac = parts2.next().unwrap_or("0");

  let frac_ms = match frac.len() {
    0 => 0,
    1 => frac.parse::<i64>().ok()? * 100,
    2 => frac.parse::<i64>().ok()? * 10,
    _ => frac.get(..3)?.parse::<i64>().ok()?,
  };

  Some(mm * 60_000 + ss * 1000 + frac_ms)
}

fn format_ms_to_ts(ms: i64) -> String {
  let mut ms = ms;
  if ms < 0 {
    ms = 0;
  }
  let total_seconds = ms / 1000;
  let mm = total_seconds / 60;
  let ss = total_seconds % 60;
  let cs = (ms % 1000) / 10; // centiseconds
  format!("[{:02}:{:02}.{:02}]", mm, ss, cs)
}

fn parse_lrc(input: &str) -> Vec<LrcLine> {
  let mut out = Vec::new();
  for line in input.lines() {
    let l = line.trim();
    if !l.starts_with('[') {
      continue;
    }
    if let Some(end) = l.find(']') {
      let ts = &l[..=end];
      let text = l[end + 1..].trim().to_string();
      if text.is_empty() {
        continue;
      }
      if let Some(ms) = parse_ts_to_ms(ts) {
        out.push(LrcLine { ms, text });
      }
    }
  }
  out.sort_by_key(|x| x.ms);
  out
}

fn build_chant_set(lines: &[LrcLine]) -> HashSet<String> {
  let mut counts: HashMap<String, usize> = HashMap::new();
  for l in lines {
    let key = normalize_text_key(&l.text);
    if key.is_empty() {
      continue;
    }
    *counts.entry(key).or_insert(0) += 1;
  }

  let mut chant = HashSet::new();
  for (k, c) in counts {
    // chant heuristic: repeated short lines
    if c >= 3 && word_count(&k) <= 4 {
      chant.insert(k);
    }
  }
  chant
}

fn find_nearest_within(
  lines: &[LrcLine],
  target_ms: i64,
  tol_ms: i64,
  used: &HashSet<usize>,
) -> Option<usize> {
  let mut best: Option<(usize, i64)> = None; // (idx, abs_diff)
  for (i, l) in lines.iter().enumerate() {
    if used.contains(&i) {
      continue;
    }
    let d = (l.ms - target_ms).abs();
    if d <= tol_ms {
      match best {
        None => best = Some((i, d)),
        Some((_, bd)) if d < bd => best = Some((i, d)),
        _ => {}
      }
    }
  }
  best.map(|(i, _)| i)
}


fn normalize_lrc_timestamps(input: &str, min_gap_ms: i64) -> String {
  let mut lines = parse_lrc(input);
  if lines.is_empty() {
    return String::new();
  }

  let mut last_ms = lines[0].ms;
  for i in 1..lines.len() {
    if lines[i].ms < last_ms {
      lines[i].ms = last_ms;
    }
    if lines[i].ms - last_ms < min_gap_ms {
      lines[i].ms = last_ms + min_gap_ms;
    }
    last_ms = lines[i].ms;
  }

  let mut out = String::new();
  for l in lines {
    out.push_str(&format_ms_to_ts(l.ms));
    out.push(' ');
    out.push_str(l.text.trim());
    out.push('\n');
  }
  out
}

fn merge_hybrid_plus(small_clean: &str, medium_clean: &str) -> String {
  let small = parse_lrc(small_clean);
  let medium = parse_lrc(medium_clean);

  if small.is_empty() {
    return normalize_lrc_timestamps(medium_clean, 250);
  }
  if medium.is_empty() {
    return normalize_lrc_timestamps(small_clean, 250);
  }

  let chant = build_chant_set(&small);

  let tol_ms = 300;
  let min_gap_ms = 250;

  let mut used_medium: HashSet<usize> = HashSet::new();
  let mut merged: Vec<LrcLine> = Vec::new();

  // baseline: small order (coverage)
  for s in &small {
    let s_key = normalize_text_key(&s.text);
    let is_chant = chant.contains(&s_key);

    if let Some(idx) = find_nearest_within(&medium, s.ms, tol_ms, &used_medium) {
      let m = &medium[idx];
      let chosen_text = if is_chant {
        // keep small for chants to preserve repetition coverage
        s.text.clone()
      } else {
        // prefer medium wording when available
        m.text.clone()
      };

      used_medium.insert(idx);
      merged.push(LrcLine {
        ms: s.ms,
        text: chosen_text,
      });
    } else {
      merged.push(s.clone());
    }
  }

  // append medium-only lines (avoid chant spam)
  for (i, m) in medium.iter().enumerate() {
    if used_medium.contains(&i) {
      continue;
    }
    let k = normalize_text_key(&m.text);
    if chant.contains(&k) {
      continue;
    }
    merged.push(m.clone());
  }

  merged.sort_by_key(|x| x.ms);

  // drop exact duplicates
  let mut dedup: Vec<LrcLine> = Vec::new();
  for l in merged {
    if let Some(last) = dedup.last() {
      if last.ms == l.ms && normalize_text_key(&last.text) == normalize_text_key(&l.text) {
        continue;
      }
    }
    dedup.push(l);
  }

  // normalize timestamps (monotonic + minimum gap)
  let mut last_ms = dedup[0].ms;
  for i in 1..dedup.len() {
    if dedup[i].ms < last_ms {
      dedup[i].ms = last_ms;
    }
    if dedup[i].ms - last_ms < min_gap_ms {
      dedup[i].ms = last_ms + min_gap_ms;
    }
    last_ms = dedup[i].ms;
  }

  let mut out = String::new();
  for l in dedup {
    out.push_str(&format_ms_to_ts(l.ms));
    out.push(' ');
    out.push_str(l.text.trim());
    out.push('\n');
  }
  out
}

/* -------------------- Cleaning -------------------- */

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
