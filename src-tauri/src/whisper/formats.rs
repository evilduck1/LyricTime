use super::linebreak::TimedLine;

pub fn to_lrc(lines: &[TimedLine]) -> String {
  let mut out = String::new();
  for l in lines {
    out.push_str(&format!("[{}]{}\n", fmt_lrc_time(l.start_ms), l.text));
  }
  out
}

fn fmt_lrc_time(ms: u64) -> String {
  // [mm:ss.xx] where xx is centiseconds
  let total_cs = ms / 10;
  let cs = total_cs % 100;
  let total_s = total_cs / 100;
  let s = total_s % 60;
  let m = total_s / 60;
  format!("{:02}:{:02}.{:02}", m, s, cs)
}

