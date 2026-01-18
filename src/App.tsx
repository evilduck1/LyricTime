import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";

type ProgressEvent =
  | { kind: "stage"; stage: string; detail?: string }
  | { kind: "log"; line: string }
  | { kind: "done"; outputPath: string };

type Model = "hybrid" | "small" | "medium";

export default function App() {
  const [audioPath, setAudioPath] = useState<string>("");
  const [model, setModel] = useState<Model>("hybrid");
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("Idle");
  const [log, setLog] = useState<string[]>([]);
  const [outputPath, setOutputPath] = useState<string>("");

  useEffect(() => {
    let unlisten: (() => void) | null = null;

    (async () => {
      unlisten = await listen<ProgressEvent>("lyric_progress", (event) => {
        const p = event.payload;
        if (p.kind === "stage") {
          setStatus(p.detail ? `${p.stage}: ${p.detail}` : p.stage);
        } else if (p.kind === "log") {
          setLog((l) => [...l.slice(-400), p.line]);
        } else if (p.kind === "done") {
          setOutputPath(p.outputPath);
          setStatus("Done");
          setBusy(false);
        }
      });
    })();

    return () => {
      if (unlisten) unlisten();
    };
  }, []);


  useEffect(() => {
    let unlisten: (() => void) | null = null;

    (async () => {
      unlisten = await listen<{
        group: string;
        file: string;
        downloaded_bytes: number;
        total_bytes: number | null;
        status: "downloading" | "done" | "error";
        error?: string | null;
      }>("download://progress", (event) => {
        const p = event.payload;

        const fmt = (bytes: number) => {
          const units = ["B", "KB", "MB", "GB", "TB"];
          let v = bytes;
          let i = 0;
          while (v >= 1024 && i < units.length - 1) {
            v /= 1024;
            i++;
          }
          return `${v.toFixed(i === 0 ? 0 : 1)}${units[i]}`;
        };

        if (p.status === "downloading") {
          const left = fmt(p.downloaded_bytes);
          const right = p.total_bytes ? fmt(p.total_bytes) : "?";
          setStatus(`Downloading ${p.file}: ${left} / ${right}`);
        }

        if (p.status === "done") {
          setStatus(`Downloaded ${p.file}`);
        }

        if (p.status === "error") {
          setStatus(`Error downloading ${p.file}`);
          setLog((l) => [...l, p.error ?? "Unknown download error"]);
        }
      });
    })();

    return () => {
      if (unlisten) unlisten();
    };
  }, []);


  const canRun = useMemo(() => !!audioPath && !busy, [audioPath, busy]);

  async function chooseFile() {
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [
        {
          name: "Audio",
          extensions: [
            "mp3",
            "m4a",
            "aac",
            "flac",
            "wav",
            "ogg",
            "opus",
            "aiff",
            "aif",
          ],
        },
      ],
    });

    if (typeof selected === "string") {
      setAudioPath(selected);
      setStatus("Ready");
      setLog([]);
      setOutputPath("");
    }
  }

  async function generate() {
    if (!canRun) return;

    setBusy(true);
    setStatus("Starting…");
    setLog([]);
    setOutputPath("");

    try {
      const out: string = await invoke("generate_lrc_next_to_audio", {
        audioPath,
        model,
      });
      setOutputPath(out);
      setBusy(false);
      setStatus("Done");
    } catch (err) {
      setBusy(false);
      setStatus("Error");
      setLog((l) => [...l, String(err)]);
    }
  }

  return (
    <div style={page()}>
      <h1 style={{ margin: 0 }}>LyricTime</h1>
      <p style={{ opacity: 0.8, marginTop: 6 }}>
        Offline line-timed lyrics generator (.lrc)
      </p>

      <div style={row()}>
        <button onClick={chooseFile} disabled={busy} style={btn()}>
          Choose audio file
        </button>

        <select
          value={model}
          onChange={(e) => setModel(e.target.value as Model)}
          disabled={busy}
          style={select()}
        >
          <option value="hybrid">Model: hybrid (best overall)</option>
          <option value="small">Model: small (fast & complete)</option>
          <option value="medium">Model: medium (best accuracy, may miss lines)</option>
        </select>

        <button
          onClick={generate}
          disabled={!canRun}
          style={btn(canRun ? "primary" : "disabled")}
        >
          {busy ? "Working…" : "Generate .lrc"}
        </button>
      </div>

      <Section title="Selected audio">{audioPath || "—"}</Section>
      <Section title="Status">{status}</Section>
      <Section title="Output">{outputPath || "—"}</Section>

      <div style={{ marginTop: 16 }}>
        <div style={label()}>Log</div>
        <pre style={logBox()}>{log.join("\n") || "—"}</pre>
      </div>
    </div>
  );
}

/* ---------- UI helpers ---------- */

function Section(props: { title: string; children: string }) {
  return (
    <div style={{ marginTop: 14 }}>
      <div style={label()}>{props.title}</div>
      <div style={monoBox()}>{props.children}</div>
    </div>
  );
}

function page(): React.CSSProperties {
  return {
    padding: 20,
    background: "#0b0b0b",
    color: "#f2f2f2",
    minHeight: "100vh",
    fontFamily:
      "system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif",
  };
}

function row(): React.CSSProperties {
  return {
    display: "flex",
    gap: 10,
    alignItems: "center",
    flexWrap: "wrap",
    marginTop: 12,
  };
}

function btn(
  variant: "primary" | "disabled" | "default" = "default"
): React.CSSProperties {
  const base: React.CSSProperties = {
    padding: "10px 14px",
    borderRadius: 10,
    border: "1px solid #2a2a2a",
    background: "#151515",
    color: "#f2f2f2",
    cursor: "pointer",
  };

  if (variant === "primary") {
    return {
      ...base,
      background: "#ffffff",
      color: "#000000",
      borderColor: "#ffffff",
    };
  }

  if (variant === "disabled") {
    return {
      ...base,
      opacity: 0.4,
      cursor: "not-allowed",
    };
  }

  return base;
}

function select(): React.CSSProperties {
  return {
    padding: "10px 12px",
    borderRadius: 10,
    border: "1px solid #2a2a2a",
    background: "#151515",
    color: "#f2f2f2",
  };
}

function label(): React.CSSProperties {
  return {
    fontSize: 13,
    opacity: 0.6,
  };
}

function monoBox(): React.CSSProperties {
  return {
    marginTop: 6,
    padding: 12,
    borderRadius: 10,
    border: "1px solid #222",
    background: "#111",
    color: "#eaeaea",
    fontFamily:
      "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
    fontSize: 12,
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
  };
}

function logBox(): React.CSSProperties {
  return {
    marginTop: 6,
    padding: 12,
    height: 220,
    overflow: "auto",
    borderRadius: 10,
    border: "1px solid #222",
    background: "#0e0e0e",
    color: "#cfcfcf",
    fontFamily:
      "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
    fontSize: 12,
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
  };
}
