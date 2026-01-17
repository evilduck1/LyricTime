use std::path::Path;

#[derive(Debug, Clone)]
pub struct Segment {
  pub start_ms: u64,
  pub end_ms: u64,
  pub text: String,
}

pub fn read_whispercpp_json(path: &Path) -> Result<Vec<Segment>, String> {
  let raw = std::fs::read_to_string(path).map_err(|e| format!("Read JSON failed: {e}"))?;
  let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("JSON parse failed: {e}"))?;

  // Find the first array anywhere in the JSON that looks like "segments".
  let segs = find_segments_array(&v).ok_or_else(|| {
    let top_keys = v
      .as_object()
      .map(|o| o.keys().cloned().collect::<Vec<_>>().join(", "))
      .unwrap_or_else(|| "(not an object)".to_string());

    format!("Could not locate a segments-like array in JSON. Top-level keys: {top_keys}")
  })?;

  let mut out = Vec::with_capacity(segs.len());

  for s in segs {
    if !s.is_object() {
      continue;
    }

    let text = s
      .get("text")
      .and_then(|t| t.as_str())
      .unwrap_or("")
      .trim()
      .to_string();

    if text.is_empty() {
      continue;
    }

    let (start_ms, end_ms) = if s.get("t0").is_some() && s.get("t1").is_some() {
      // centiseconds -> ms (common whisper.cpp format)
      let t0 = s.get("t0").and_then(|n| n.as_i64()).unwrap_or(0).max(0) as u64;
      let t1 = s.get("t1").and_then(|n| n.as_i64()).unwrap_or(0).max(0) as u64;
      (t0 * 10, t1 * 10)
    } else if s.get("start").is_some() && s.get("end").is_some() {
      // seconds -> ms (common alternative)
      let start = s.get("start").and_then(|n| n.as_f64()).unwrap_or(0.0).max(0.0);
      let end = s.get("end").and_then(|n| n.as_f64()).unwrap_or(start).max(start);
      ((start * 1000.0) as u64, (end * 1000.0) as u64)
    } else {
      // Unknown timing format for this entry
      continue;
    };

    out.push(Segment { start_ms, end_ms, text });
  }

  if out.is_empty() {
    return Err("Found a segments-like array, but parsed zero usable segments (no timing/text).".into());
  }

  Ok(out)
}

// Recursively search JSON for an array whose elements look like whisper segments.
// A "segment-like" object has `text` and either (`t0`+`t1`) or (`start`+`end`).
fn find_segments_array<'a>(v: &'a serde_json::Value) -> Option<&'a Vec<serde_json::Value>> {
  match v {
    serde_json::Value::Array(arr) => {
      if looks_like_segments_array(arr) {
        return Some(arr);
      }
      // Search inside elements
      for item in arr {
        if let Some(found) = find_segments_array(item) {
          return Some(found);
        }
      }
      None
    }
    serde_json::Value::Object(map) => {
      // First: if there is a direct "segments" key, check it.
      if let Some(s) = map.get("segments") {
        if let serde_json::Value::Array(arr) = s {
          if looks_like_segments_array(arr) {
            return Some(arr);
          }
        }
      }

      // Otherwise: search all values recursively.
      for (_k, val) in map {
        if let Some(found) = find_segments_array(val) {
          return Some(found);
        }
      }
      None
    }
    _ => None,
  }
}

fn looks_like_segments_array(arr: &Vec<serde_json::Value>) -> bool {
  // Need at least one object that matches the segment pattern.
  for v in arr.iter().take(10) {
    if let serde_json::Value::Object(m) = v {
      let has_text = m.get("text").and_then(|t| t.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false);

      let has_t0t1 = m.get("t0").is_some() && m.get("t1").is_some();
      let has_startend = m.get("start").is_some() && m.get("end").is_some();

      if has_text && (has_t0t1 || has_startend) {
        return true;
      }
    }
  }
  false
}

