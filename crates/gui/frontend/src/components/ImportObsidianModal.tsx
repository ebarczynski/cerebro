import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

// ── Types ──────────────────────────────────────────────────────

export interface VaultImportSummary {
  imported: number;
  skipped: number;
  failed: number;
  total_found: number;
  errors: string[];
}

interface ImportObsidianModalProps {
  onClose: () => void;
  /** Called once with the summary after a (partial) import — parent reloads notes + toasts. */
  onImported: (summary: VaultImportSummary) => void;
}

type Phase = "pick" | "ready" | "importing" | "done";

// ── Component ──────────────────────────────────────────────────

export default function ImportObsidianModal({ onClose, onImported }: ImportObsidianModalProps) {
  const [phase, setPhase] = useState<Phase>("pick");
  const [vaultPath, setVaultPath] = useState<string | null>(null);
  const [folder, setFolder] = useState("Obsidian");
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [summary, setSummary] = useState<VaultImportSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const unlistenRef = useRef<(() => void) | null>(null);

  // Clean up the progress listener whenever it exists
  useEffect(() => {
    return () => {
      unlistenRef.current?.();
    };
  }, []);

  const pickDirectory = useCallback(async () => {
    setError(null);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Select your Obsidian vault folder",
      });
      if (!selected) return;
      setVaultPath(typeof selected === "string" ? selected : String(selected));
      setPhase("ready");
    } catch (e) {
      setError(String(e).replace(/^Error:\s*/, ""));
    }
  }, []);

  const startImport = useCallback(async () => {
    if (!vaultPath) return;
    setError(null);
    setPhase("importing");
    setProgress(null);

    try {
      // Listen for backend progress events during the walk.
      try {
        const { listen } = await import("@tauri-apps/api/event");
        unlistenRef.current = await listen<{ done: number; total: number }>(
          "import-progress",
          (event) => setProgress(event.payload),
        );
      } catch {
        // Progress events are optional — import still works without them.
      }

      const result = await invoke<VaultImportSummary>("import_obsidian_vault", {
        vaultPath,
        folder: folder.trim() ? folder.trim() : null,
      });

      setSummary(result);
      setPhase("done");
      onImported(result);
    } catch (e) {
      setError(String(e).replace(/^Error:\s*/, ""));
      setPhase(vaultPath ? "ready" : "pick");
    } finally {
      unlistenRef.current?.();
      unlistenRef.current = null;
    }
  }, [vaultPath, folder, onImported]);

  const pct = progress && progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : 0;

  return (
    <div
      className="fixed inset-0 bg-black/70 flex items-center justify-center z-[60]"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && phase !== "importing") onClose();
      }}
    >
      <div className="bg-[#12121a] rounded-xl border border-[#2a2a3a] w-[480px] p-6 shadow-2xl animate-fade-in">
        <div className="flex items-center gap-2.5 mb-1">
          <span className="text-xl">💠</span>
          <h2 className="text-lg font-semibold text-white">Import Obsidian Vault</h2>
        </div>
        <p className="text-xs text-[#8888a0] mb-4 leading-relaxed">
          Imports every Markdown note from an Obsidian vault — preserving folder
          structure, frontmatter tags and <code className="text-[#a5b4fc]">[[wikilinks]]</code>.
          The <code className="text-[#a5b4fc]">.obsidian</code> config folder is skipped.
        </p>

        {/* Step 1 — pick a folder */}
        {phase === "pick" && (
          <button
            onClick={pickDirectory}
            className="w-full px-4 py-3 rounded-lg border-2 border-dashed border-[#2a2a3a] hover:border-[#6366f1] hover:bg-[#6366f1]/10 text-sm text-[#a5b4fc] transition-colors"
          >
            📂 Choose your Obsidian vault folder…
          </button>
        )}

        {/* Step 2 — confirm + destination folder */}
        {(phase === "ready" || phase === "importing") && vaultPath && (
          <div className="space-y-3">
            <div className="flex items-start gap-2 p-2.5 rounded-lg bg-[#1a1a25] border border-[#2a2a3a]">
              <span className="text-[#8888a0] mt-0.5">📁</span>
              <span
                className="text-xs text-[#c8c8e0] break-all"
                style={{ fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace" }}
              >
                {vaultPath}
              </span>
            </div>

            <div>
              <label className="block text-xs font-semibold uppercase tracking-wider text-[#8888a0] mb-1.5">
                Destination folder (optional)
              </label>
              <input
                value={folder}
                onChange={(e) => setFolder(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && phase === "ready") startImport();
                }}
                disabled={phase === "importing"}
                placeholder="Obsidian — leave empty for vault root"
                className="w-full px-3 py-2 bg-[#0a0a0f] border border-[#2a2a3a] rounded-lg text-sm text-[#e4e4f0] placeholder-[#6b6b85] focus:outline-none focus:border-[#6366f1] focus:ring-1 focus:ring-[#6366f1]/40 transition-colors disabled:opacity-50"
              />
              <p className="text-[10px] text-[#71717a] mt-1.5">
                Notes land under this folder so they never clash with existing notes.
              </p>
            </div>

            {/* Progress bar */}
            {phase === "importing" && (
              <div className="pt-1">
                <div className="flex items-center justify-between text-[11px] text-[#8888a0] mb-1.5">
                  <span>Importing notes…</span>
                  <span style={{ fontFamily: "ui-monospace, monospace" }}>
                    {progress ? `${progress.done}/${progress.total}` : "scanning…"}
                  </span>
                </div>
                <div className="h-1.5 rounded-full bg-[#1a1a25] overflow-hidden">
                  <div
                    className="h-full bg-[#6366f1] rounded-full transition-all duration-200"
                    style={{ width: `${pct}%` }}
                  />
                </div>
              </div>
            )}

            {error && (
              <p className="text-xs" style={{ color: "#f87171" }}>
                {error}
              </p>
            )}

            <div className="flex justify-end gap-2 pt-1">
              <button
                onClick={onClose}
                disabled={phase === "importing"}
                className="px-4 py-2 text-sm rounded-lg bg-[#1e1e2e] hover:bg-[#2a2a3e] text-[#a0a0b8] border border-[#333] disabled:opacity-40"
              >
                {phase === "importing" ? "Importing…" : "Cancel"}
              </button>
              {phase === "ready" && (
                <button
                  onClick={startImport}
                  className="px-4 py-2 text-sm rounded-lg font-medium text-white transition-colors"
                  style={{ backgroundColor: "#6366f1" }}
                >
                  Import Vault
                </button>
              )}
            </div>
          </div>
        )}

        {/* Step 3 — summary */}
        {phase === "done" && summary && (
          <div className="space-y-3">
            <div className="grid grid-cols-3 gap-2 text-center">
              <div className="p-3 rounded-lg bg-[#1a1a25] border border-[#2a2a3a]">
                <p className="text-xl font-semibold" style={{ color: "#4ade80" }}>{summary.imported}</p>
                <p className="text-[10px] uppercase tracking-wider text-[#8888a0]">Imported</p>
              </div>
              <div className="p-3 rounded-lg bg-[#1a1a25] border border-[#2a2a3a]">
                <p className="text-xl font-semibold text-[#eab308]">{summary.skipped}</p>
                <p className="text-[10px] uppercase tracking-wider text-[#8888a0]">Skipped</p>
              </div>
              <div className="p-3 rounded-lg bg-[#1a1a25] border border-[#2a2a3a]">
                <p className="text-xl font-semibold" style={{ color: "#f87171" }}>{summary.failed}</p>
                <p className="text-[10px] uppercase tracking-wider text-[#8888a0]">Failed</p>
              </div>
            </div>

            <p className="text-[11px] text-[#8888a0]">
              {summary.total_found} Markdown file{summary.total_found === 1 ? "" : "s"} found.
              {summary.skipped > 0 && " Skipped files already existed at their destination path."}
            </p>

            {summary.errors.length > 0 && (
              <div className="max-h-28 overflow-auto rounded-lg bg-[#1a0a0a] border border-[#ef444430] p-2.5">
                {summary.errors.map((err, i) => (
                  <p key={i} className="text-[10px] text-[#fca5a5] break-all" style={{ fontFamily: "ui-monospace, monospace" }}>
                    {err}
                  </p>
                ))}
              </div>
            )}

            <div className="flex justify-end pt-1">
              <button
                onClick={onClose}
                className="px-4 py-2 text-sm rounded-lg font-medium text-white"
                style={{ backgroundColor: "#6366f1" }}
              >
                Done
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
