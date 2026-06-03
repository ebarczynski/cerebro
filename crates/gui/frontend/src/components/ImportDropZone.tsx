import { useState, useCallback, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

// ── Types ──────────────────────────────────────────────────────

interface ImportResponse {
  title: string;
  format: string;
  tags: string[];
  note_id: string;
  note_path: string;
}

interface ImportDropZoneProps {
  onImportComplete: (responses: ImportResponse[]) => void;
  onError: (message: string) => void;
  vaultOpen: boolean;
}

// Supported file extensions — documents + text/code files
const DOCUMENT_EXTENSIONS = new Set(["pdf", "pptx", "docx", "doc", "ods", "xlsx", "xls", "odt", "html", "htm"]);
const TEXT_EXTENSIONS = new Set([
  "md", "markdown", "txt", "text", "csv", "tsv",
  "json", "jsonl", "yaml", "yml", "toml", "ini", "cfg",
  "xml", "html", "htm", "css", "scss", "less",
  "log", "rst", "asciidoc", "adoc",
  "js", "ts", "jsx", "tsx", "py", "rb", "rs", "go", "java", "c", "cpp", "h", "hpp",
  "sh", "bash", "zsh", "fish",
  "sql", "graphql",
  "dockerfile", "makefile",
]);
const ALL_SUPPORTED = new Set([...DOCUMENT_EXTENSIONS, ...TEXT_EXTENSIONS]);

function isSupportedFile(path: string): boolean {
  const ext = path.split(".").pop()?.toLowerCase() || "";
  // Also match files with no extension but known names
  const basename = path.split("/").pop()?.toLowerCase() || "";
  return ALL_SUPPORTED.has(ext) || ["readme", "license", "makefile", "dockerfile", ".gitignore"].includes(basename);
}

function isTextFile(path: string): boolean {
  const ext = path.split(".").pop()?.toLowerCase() || "";
  return TEXT_EXTENSIONS.has(ext);
}

function fileExtension(path: string): string {
  return path.split(".").pop()?.toLowerCase() || "";
}

// ── Component ──────────────────────────────────────────────────

export default function ImportDropZone({
  onImportComplete,
  onError,
  vaultOpen,
}: ImportDropZoneProps) {
  const [isDragging, setIsDragging] = useState(false);
  const [importing, setImporting] = useState(false);
  const [dragFiles, setDragFiles] = useState<string[]>([]);
  const [progress, setProgress] = useState("");

  // ── Handle file import ─────────────────────────────────────

  const handleImport = useCallback(
    async (filePaths: string[]) => {
      if (filePaths.length === 0) return;

      const supported = filePaths.filter(isSupportedFile);
      if (supported.length === 0) {
        onError(
          "No supported files found. Supported: MD, TXT, JSON, CSV, YAML, PDF, DOCX, PPTX, and more"
        );
        return;
      }

      setImporting(true);
      setProgress(`Importing ${supported.length} file(s)...`);

      try {
        const results = await invoke<ImportResponse[]>("import_documents", {
          filePaths: supported,
          folder: null,
        });

        setProgress(
          `✅ Imported ${results.length} file(s): ${results
            .map((r) => `${r.title} (${r.format})`)
            .join(", ")}`
        );
        onImportComplete(results);

        // Clear progress after 4 seconds
        setTimeout(() => setProgress(""), 4000);
      } catch (e) {
        onError(`Import failed: ${e}`);
        setProgress("");
      } finally {
        setImporting(false);
        setDragFiles([]);
      }
    },
    [onImportComplete, onError]
  );

  // ── File picker (button click) ─────────────────────────────

  const handlePickFiles = useCallback(async () => {
    if (!vaultOpen || importing) return;

    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        multiple: true,
        filters: [
          {
            name: "All Supported",
            extensions: [
              "md", "txt", "json", "csv", "yaml", "yml", "toml",
              "pdf", "pptx", "docx", "doc", "ods", "xlsx", "xls", "odt",
              "html", "htm", "xml", "rst",
              "py", "js", "ts", "rs", "go", "java", "c", "cpp", "sh",
              "sql", "log",
            ],
          },
          { name: "Markdown & Text", extensions: ["md", "txt", "text", "csv", "rst"] },
          { name: "Documents", extensions: ["pdf", "pptx", "docx", "doc", "ods", "xlsx", "xls", "odt", "html"] },
        ],
        title: "Select files to import",
      });

      if (!selected) return;

      const paths = selected as string[];

      await handleImport(paths);
    } catch (e) {
      onError(`File picker error: ${e}`);
    }
  }, [vaultOpen, importing, handleImport, onError]);

  // ── Drag-and-drop via HTML5 events ─────────────────────────
  // Tauri 2 forwards native OS file drops as HTML5 drag events

  useEffect(() => {
    const handleDragEnter = (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (!vaultOpen) return;
      setIsDragging(true);
    };

    const handleDragOver = (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
    };

    const handleDragLeave = (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      // Only set false if leaving the window entirely
      const rect = document.documentElement.getBoundingClientRect();
      if (
        e.clientX <= rect.left ||
        e.clientX >= rect.right ||
        e.clientY <= rect.top ||
        e.clientY >= rect.bottom
      ) {
        setIsDragging(false);
        setDragFiles([]);
      }
    };

    const handleDrop = async (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setIsDragging(false);
      setDragFiles([]);

      if (!vaultOpen) return;

      // In Tauri 2, dropped files appear in dataTransfer.files
      const files = e.dataTransfer?.files;
      if (!files || files.length === 0) return;

      // We can't read file paths from HTML5 File objects in Tauri directly,
      // so we use the Tauri event system instead via @tauri-apps/api
      // The file-drop event gives us paths
    };

    document.addEventListener("dragenter", handleDragEnter);
    document.addEventListener("dragover", handleDragOver);
    document.addEventListener("dragleave", handleDragLeave);
    document.addEventListener("drop", handleDrop);

    return () => {
      document.removeEventListener("dragenter", handleDragEnter);
      document.removeEventListener("dragover", handleDragOver);
      document.removeEventListener("dragleave", handleDragLeave);
      document.removeEventListener("drop", handleDrop);
    };
  }, [vaultOpen]);

  // ── Tauri 2 drag-drop events ───────────────────────────────
  // Use the Tauri webview drag-drop event listener for actual file paths

  useEffect(() => {
    let unlisten: (() => void) | null = null;

    const setup = async () => {
      try {
        const { getCurrentWebview } = await import(
          "@tauri-apps/api/webview"
        );
        const webview = getCurrentWebview();

        unlisten = await webview.onDragDropEvent((event: any) => {
          if (event.payload.type === "over") {
            setIsDragging(true);
            if (event.payload.paths) {
              setDragFiles(
                event.payload.paths.filter((p: string) => isSupportedFile(p))
              );
            }
          } else if (event.payload.type === "drop") {
            setIsDragging(false);
            setDragFiles([]);
            if (event.payload.paths && event.payload.paths.length > 0) {
              handleImport(event.payload.paths);
            }
          } else if (event.payload.type === "leave") {
            setIsDragging(false);
            setDragFiles([]);
          }
        });
      } catch {
        // webview API may not be available in dev mode — fallback to HTML5 events
        console.warn(
          "Tauri webview drag-drop not available, using HTML5 fallback"
        );
      }
    };

    setup();
    return () => {
      if (unlisten) unlisten();
    };
  }, [handleImport]);

  // ── Render ──────────────────────────────────────────────────

  if (!vaultOpen) return null;

  return (
    <>
      {/* Import button in toolbar — rendered by parent */}
      <button
        onClick={handlePickFiles}
        disabled={importing}
        className="p-1.5 hover:bg-[#1a1a25] rounded text-[#8888a0] hover:text-white transition-colors disabled:opacity-50"
        title="Import files (MD, TXT, JSON, CSV, PDF, DOCX, PPTX, and more)"
      >
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
        >
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
          <polyline points="7 10 12 15 17 10" />
          <line x1="12" y1="15" x2="12" y2="3" />
        </svg>
      </button>

      {/* Drop overlay */}
      {isDragging && (
        <div className="fixed inset-0 z-50 pointer-events-none">
          <div className="absolute inset-4 border-2 border-dashed border-[#6366f1] rounded-2xl bg-[#6366f1]/10 backdrop-blur-sm flex items-center justify-center">
            <div className="text-center">
              <div className="text-6xl mb-4 animate-bounce">📥</div>
              <h2 className="text-2xl font-bold text-white mb-2">
                Drop files to import
              </h2>
              <p className="text-[#8888a0] text-sm">
                Supports MD, TXT, JSON, CSV, YAML, PDF, DOCX, PPTX & more
              </p>
              {dragFiles.length > 0 && (
                <div className="mt-4 space-y-1">
                  {dragFiles.map((f, i) => (
                    <p key={i} className="text-[#818cf8] text-xs font-mono">
                      📄 {f.split("/").pop() || f.split("\\").pop() || f}
                    </p>
                  ))}
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Import progress toast */}
      {(importing || progress) && (
        <div className="fixed bottom-14 left-1/2 -translate-x-1/2 z-40">
          <div className="bg-[#1e1e2e] border border-[#2a2a3a] rounded-lg px-4 py-2 shadow-xl flex items-center gap-2 max-w-lg">
            {importing ? (
              <div className="animate-spin h-4 w-4 border-2 border-[#6366f1] border-t-transparent rounded-full" />
            ) : (
              <span className="text-green-400">✓</span>
            )}
            <span className="text-sm text-white truncate">{progress}</span>
          </div>
        </div>
      )}
    </>
  );
}

export type { ImportResponse };
export { isSupportedFile };
