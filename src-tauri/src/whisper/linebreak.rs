use super::parse::Segment;

#[derive(Debug, Clone)]
pub struct TimedLine {
  pub start_ms: u64,
  pub end_ms: u64,
  pub text: String,
}

pub fn segments_to_lines(segments: &[Segment]) -> Vec<TimedLine> {
  let mut lines: Vec<TimedLine> = Vec::new();

  let mut cur_start: Option<u64> = None;
  let mut cur_end: u64 = 0;
  let mut cur_text = String::new();
  let mut last_end: Option<u64> = None;

  for seg in segments {
    let pause_ms = last_end
      .map(|e| seg.start_ms.saturating_sub(e))
      .unwrap_or(0);

    let seg_text = normalize_spaces(&seg.text);

    if cur_start.is_none() {
      cur_start = Some(seg.start_ms);
      cur_end = seg.end_ms;
      cur_text = seg_text;
    } else {
      let cur_len = cur_text.len();
      let cur_dur = cur_end.saturating_sub(cur_start.unwrap_or(cur_end));
      let ends_with_punct = cur_text.trim_end().ends_with(['.', '!', '?', ',', ';', ':']);

      let should_break =
        pause_ms > 650 ||
        ends_with_punct ||
        cur_len > 64 ||
        cur_dur > 4500;

      if should_break {
        lines.push(TimedLine {
          start_ms: cur_start.unwrap(),
          end_ms: cur_end,
          text: cur_text.trim().to_string(),
        });

        cur_start = Some(seg.start_ms);
        cur_end = seg.end_ms;
        cur_text = seg_text;
      } else {
        cur_end = seg.end_ms;
        if !cur_text.ends_with(' ') {
          cur_text.push(' ');
        }
        cur_text.push_str(&seg_text);
      }
    }

    last_end = Some(seg.end_ms);
  }

  if let Some(s) = cur_start {
    let t = cur_text.trim().to_string();
    if !t.is_empty() {
      lines.push(TimedLine { start_ms: s, end_ms: cur_end, text: t });
    }
  }

  merge_tiny(lines)
}

fn normalize_spaces(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  let mut last_space = false;

  for ch in s.chars() {
    let is_space = ch.is_whitespace();
    if is_space {
      if !last_space {
        out.push(' ');
      }
      last_space = true;
    } else {
      out.push(ch);
      last_space = false;
    }
  }

  out.trim().to_string()
}

fn merge_tiny(lines: Vec<TimedLine>) -> Vec<TimedLine> {
  if lines.len() < 2 {
    return lines;
  }

  let mut out: Vec<TimedLine> = Vec::with_capacity(lines.len());
  let mut i = 0;

  while i < lines.len() {
    let cur = &lines[i];
    let word_count = cur.text.split_whitespace().count();
    let tiny = word_count <= 2 && (cur.end_ms.saturating_sub(cur.start_ms) <= 1200);

    if tiny && i + 1 < lines.len() {
      let mut next = lines[i + 1].clone();
      next.start_ms = cur.start_ms;
      next.text = format!("{} {}", cur.text, next.text).trim().to_string();
      out.push(next);
      i += 2;
    } else {
      out.push(cur.clone());
      i += 1;
    }
  }

  out
}

