import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { homeDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import Sidebar from "./components/Sidebar";
import Editor from "./components/Editor";
import SearchModal from "./components/SearchModal";
import StatusBar from "./components/StatusBar";
import ImportDropZone, { type ImportResponse } from "./components/ImportDropZone";

// ── Types ──────────────────────────────────────────────────────

interface Note {
  id: string;
  title: string;
  path: string;
  content_hash?: string;
  frontmatter?: string | null;
  tags: string[];
  word_count?: number;
  created_at?: string;
  updated_at: string;
  category?: string | null;
  is_favorite?: boolean;
}

interface NoteContent {
  note: Note;
  body: string;
}

interface SearchResult {
  title: string;
  path: string;
  rank: number;
  snippet: string;
  tags: string[];
}

interface TagInfo {
  name: string;
  count: number;
}

interface ProfileInfo {
  name: string;
  display_name: string;
  is_active: boolean;
  notes_dir: string;
  encryption: boolean;
}

type ViewMode = "edit" | "preview" | "split";

// ── Simple Markdown Preview ────────────────────────────────────

function MarkdownPreview({ content }: { content: string }) {
  const [html, setHtml] = useState("");

  useEffect(() => {
    const loadMarkdown = async () => {
      const [{ marked }, hljsModule] = await Promise.all([
        import("marked"),
        import("highlight.js").catch(() => null),
      ]);
      const hljs = (hljsModule as any)?.default ?? null;
      try {
        // Custom image renderer for vault attachments
        const renderer = new marked.Renderer();

        // Store vault image paths for async loading after initial parse
        const vaultImagePaths: string[] = [];
        const vaultExcalidrawPaths: string[] = [];

        const originalImage = renderer.image.bind(renderer);
        renderer.image = function(token: any) {
          const href: string = token.href || "";
          if (href.endsWith('.excalidraw')) {
            vaultExcalidrawPaths.push(href);
            return `<div class="excalidraw-placeholder" data-excalidraw-path="${href}">Loading diagram...</div>`;
          }
          if (href.startsWith("attachments/")) {
            vaultImagePaths.push(href);
            return `<img src="${href}" alt="${token.text || ""}" loading="lazy" class="vault-image" data-vault-path="${href}" />`;
          }
          return originalImage.call(renderer, token);
        };

        // Syntax highlighting for fenced code blocks
        renderer.code = function(token: any) {
          const code = token.text || "";
          const lang = token.lang || "";

          let highlighted: string;
          if (hljs) {
            try {
              if (lang && hljs.getLanguage(lang)) {
                highlighted = hljs.highlight(code, { language: lang }).value;
              } else {
                highlighted = hljs.highlightAuto(code).value;
              }
            } catch {
              highlighted = code;
            }
          } else {
            highlighted = code
              .replace(/&/g, "&amp;")
              .replace(/</g, "&lt;")
              .replace(/>/g, "&gt;");
          }

          const langLabel = lang ? `<span class="code-lang">${lang}</span>` : '';
          return `<div class="code-block-wrapper">${langLabel}<pre><code class="hljs language-${lang}">${highlighted}</code></pre></div>`;
        };

        const result = await marked.parse(content, { async: false, renderer } as any) as unknown as string;

        // Post-process: load vault images as base64
        let processed = result;
        if (vaultImagePaths.length > 0) {
          for (const path of vaultImagePaths) {
            try {
              const base64 = await invoke<string>("read_image_as_base64", { path });
              const ext = path.split('.').pop()?.toLowerCase();
              const mime = ext === 'png' ? 'image/png' : ext === 'gif' ? 'image/gif' : ext === 'webp' ? 'image/webp' : 'image/jpeg';
              processed = processed.replace(`src="${path}"`, `src="data:${mime};base64,${base64}"`);
            } catch (e) {
              console.error("Failed to load image:", path, e);
            }
          }
        }

        // Post-process: load excalidraw diagrams as inline SVG
        if (vaultExcalidrawPaths.length > 0) {
          const { excalidrawToSvg } = await import("./utils/excalidrawToSvg");
          for (const path of vaultExcalidrawPaths) {
            try {
              const json = await invoke<string>("read_attachment_text", { path });
              const svgHtml = excalidrawToSvg(json);
              processed = processed.replace(
                `<div class="excalidraw-placeholder" data-excalidraw-path="${path}">Loading diagram...</div>`,
                svgHtml
              );
            } catch (e) {
              console.error("Failed to load excalidraw:", path, e);
              processed = processed.replace(
                `<div class="excalidraw-placeholder" data-excalidraw-path="${path}">Loading diagram...</div>`,
                `<div style="padding:12px;color:#ef4444;font-size:13px;border:1px solid #2a2a3a;border-radius:8px;">Failed to load diagram: ${path}</div>`
              );
            }
          }
        }

        setHtml(processed);
      } catch {
        setHtml(`<pre>${content}</pre>`);
      }
    };
    loadMarkdown().catch(() => {
      setHtml(`<pre>${content}</pre>`);
    });
  }, [content]);

  return (
    <div
      className="markdown-preview p-6 overflow-auto h-full"
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}

// ── Graph View Panel ───────────────────────────────────────────

function GraphViewPanel({ onClose }: { onClose: () => void }) {
  const svgRef = useRef<SVGSVGElement>(null);

  useEffect(() => {
    Promise.all([
      invoke<{ nodes: { id: string; title: string; path: string }[]; edges: { source: string; target: string }[] }>("get_graph"),
      invoke<{ source_id: string; target_id: string; confidence: number }[]>("get_keyword_links"),
    ])
      .then(async ([data, keywordLinks]) => {
        if (!svgRef.current || data.nodes.length === 0) return;

        const d3 = await import("d3");
        const svg = d3.select(svgRef.current);
        const width = svgRef.current.clientWidth;
        const height = svgRef.current.clientHeight;

        svg.selectAll("*").remove();

        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const nodes: any[] = data.nodes.map((n) => ({ ...n }));
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const edges: any[] = data.edges.map((e) => ({ ...e, type: "wikilink" }));

        // Add keyword-based edges (avoid exact duplicates with wikilinks)
        const existingPairs = new Set(edges.map((e: any) => `${e.source}->${e.target}`));
        for (const kl of keywordLinks) {
          const fwd = `${kl.source_id}->${kl.target_id}`;
          const rev = `${kl.target_id}->${kl.source_id}`;
          if (!existingPairs.has(fwd) && !existingPairs.has(rev)) {
            edges.push({
              source: kl.source_id,
              target: kl.target_id,
              type: "keyword",
              confidence: kl.confidence,
            });
            existingPairs.add(fwd);
          }
        }

        const simulation = d3
          .forceSimulation(nodes)
          .force("link", d3.forceLink(edges).id((d: any) => d.id).distance(100))
          .force("charge", d3.forceManyBody().strength(-200))
          .force("center", d3.forceCenter(width / 2, height / 2))
          .force("collision", d3.forceCollide().radius(30));

        const link = svg
          .append("g")
          .selectAll("line")
          .data(edges)
          .join("line")
          .attr("stroke", (d: any) => d.type === "keyword" ? "#f59e0b60" : "#2a2a3a")
          .attr("stroke-width", (d: any) => d.type === "keyword" ? 1 : 1.5)
          .attr("stroke-dasharray", (d: any) => d.type === "keyword" ? "4 3" : "none");

        const drag = d3.drag<SVGSVGElement, any>()
          .on("start", (event: any, d: any) => {
            if (!event.active) simulation.alphaTarget(0.3).restart();
            d.fx = d.x;
            d.fy = d.y;
          })
          .on("drag", (event: any, d: any) => {
            d.fx = event.x;
            d.fy = event.y;
          })
          .on("end", (event: any, d: any) => {
            if (!event.active) simulation.alphaTarget(0);
            d.fx = null;
            d.fy = null;
          });

        const node = svg
          .append("g")
          .selectAll("circle")
          .data(nodes)
          .join("circle")
          .attr("r", 8)
          .attr("fill", "#6366f1")
          .attr("stroke", "#818cf8")
          .attr("stroke-width", 1.5)
          .on("click", (event: any, d: any) => {
            event.stopPropagation();
          }) as any;
        
        node.call(drag as any);

        const label = svg
          .append("g")
          .selectAll("text")
          .data(nodes)
          .join("text")
          .text((d: any) => d.title.length > 15 ? d.title.slice(0, 15) + "…" : d.title)
          .attr("font-size", 10)
          .attr("fill", "#8888a0")
          .attr("text-anchor", "middle")
          .attr("dy", -14);

        simulation.on("tick", () => {
          link
            .attr("x1", (d: any) => d.source.x)
            .attr("y1", (d: any) => d.source.y)
            .attr("x2", (d: any) => d.target.x)
            .attr("y2", (d: any) => d.target.y);
          node.attr("cx", (d: any) => d.x).attr("cy", (d: any) => d.y);
          label.attr("x", (d: any) => d.x).attr("y", (d: any) => d.y);
        });
      })
      .catch(console.error);
  }, []);

  return (
    <div className="fixed inset-0 bg-black/70 flex items-center justify-center z-50">
      <div className="bg-[#12121a] rounded-xl border border-[#2a2a3a] w-[80vw] h-[70vh] flex flex-col">
        <div className="flex items-center justify-between p-4 border-b border-[#2a2a3a]">
          <h2 className="text-lg font-semibold text-white">Knowledge Graph</h2>
          <div className="flex items-center gap-4 text-xs text-[#8888a0]">
            <span className="flex items-center gap-1">
              <span className="inline-block w-6 h-0.5 bg-[#2a2a3a]" /> Wikilinks
            </span>
            <span className="flex items-center gap-1">
              <span className="inline-block w-6 h-0.5 bg-[#f59e0b60]" style={{ borderTop: '1px dashed #f59e0b60' }} /> Keywords
            </span>
          </div>
          <button onClick={onClose} className="text-gray-400 hover:text-white text-2xl leading-none">&times;</button>
        </div>
        <svg ref={svgRef} className="flex-1 w-full" />
      </div>
    </div>
  );
}

// ── Welcome Screen ─────────────────────────────────────────────

function WelcomeScreen({ onOpen, onCreate }: { onOpen: () => void; onCreate: () => void }) {
  return (
    <div className="flex-1 flex items-center justify-center bg-[#0a0a0f]">
      <div className="text-center max-w-md">
        <div className="text-6xl mb-6">🧠</div>
        <h1 className="text-3xl font-bold text-white mb-3">Cerebro</h1>
        <p className="text-[#8888a0] mb-8 leading-relaxed">
          Your personal knowledge base. Store notes, connect ideas, and explore your knowledge graph.
        </p>
        <div className="space-y-3">
          <button
            onClick={onOpen}
            className="w-full px-6 py-3 bg-[#6366f1] hover:bg-[#818cf8] text-white rounded-lg font-medium transition-colors"
          >
            Open Existing Vault
          </button>
          <button
            onClick={onCreate}
            className="w-full px-6 py-3 bg-[#1e1e2e] hover:bg-[#2a2a3e] text-[#818cf8] border border-[#333] rounded-lg font-medium transition-colors"
          >
            Create New Vault
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Main App ───────────────────────────────────────────────────

const LAST_VAULT_KEY = 'cerebro-last-vault';

interface LastVault {
  profile: string;
  path: string;
}

export default function App() {
  // State
  const [notes, setNotes] = useState<Note[]>([]);
  const [activeNote, setActiveNote] = useState<Note | null>(null);
  const [noteContent, setNoteContent] = useState<string>("");
  const [tags, setTags] = useState<TagInfo[]>([]);
  const [profiles, setProfiles] = useState<ProfileInfo[]>([]);
  const [activeProfile, setActiveProfile] = useState<ProfileInfo | null>(null);
  const [selectedTag, setSelectedTag] = useState<string | null>(null);
  const [searchResults, setSearchResults] = useState<SearchResult[]>([]);
  const [backlinks, setBacklinks] = useState<any[]>([]);
  const [viewMode, setViewMode] = useState<ViewMode>("edit");
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [rightPanelOpen, setRightPanelOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [graphOpen, setGraphOpen] = useState(false);
  const [vaultOpen, setVaultOpen] = useState(false);
  const [wordCount, setWordCount] = useState(0);
  const [lastSaved, setLastSaved] = useState<Date | null>(null);
  const [saving, setSaving] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [editorSearchQuery, setEditorSearchQuery] = useState<string>("");
  const [toastMessage, setToastMessage] = useState<string | null>(null);

  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // ── Load vault ─────────────────────────────────────────────

  const loadVault = useCallback(async (profileName?: string, vaultPath?: string) => {
    try {
      const home = await homeDir();
      const profile = profileName || "personal";
      const path = vaultPath || `${home}.cerebro/profiles/${profile}`;

      const info = await invoke<{ name: string; path: string; note_count: number; tag_count: number }>("open_vault", {
        path,
        profile,
      });

      setVaultOpen(true);

      const [notesList, tagsList, profilesList] = await Promise.all([
        invoke<Note[]>("list_notes", { limit: 500, offset: 0 }),
        invoke<TagInfo[]>("list_tags"),
        invoke<ProfileInfo[]>("list_profiles"),
      ]);

      setNotes(notesList);
      setTags(tagsList);
      setProfiles(profilesList);

      const ap = profilesList.find((p) => p.is_active) || profilesList[0] || null;
      setActiveProfile(ap);

      // Persist last opened vault for auto-loading on next startup
      try {
        localStorage.setItem(LAST_VAULT_KEY, JSON.stringify({ profile, path }));
      } catch {}
    } catch (e) {
      console.error("Failed to load vault:", e);
      setVaultOpen(false);
    }
  }, []);

  const handleOpenVault = useCallback(async () => {
    try {
      const selected = await open({ directory: true, multiple: false, title: "Select Vault Directory" });
      if (!selected) return;
      const vaultPath = typeof selected === 'string' ? selected : selected as string;
      const profileName = vaultPath.split('/').pop() || 'personal';
      await loadVault(profileName, vaultPath);
    } catch (e) {
      console.error("Failed to open vault:", e);
    }
  }, [loadVault]);

  const handleCreateVault = useCallback(async () => {
    try {
      const name = prompt('Enter vault name:', 'my-vault');
      if (!name) return;
      const home = await homeDir();
      const vaultPath = `${home}.cerebro/profiles/${name}`;
      await invoke('create_vault', { name, path: vaultPath });
      await loadVault(name, vaultPath);
    } catch (e) {
      console.error("Failed to create vault:", e);
    }
  }, [loadVault]);

  // Auto-load on mount — try last opened vault, fall back to "personal"
  useEffect(() => {
    (async () => {
      try {
        const home = await homeDir();

        // Try to restore the last opened vault
        let profile = 'personal';
        let defaultPath = `${home}.cerebro/profiles/personal`;
        try {
          const stored = localStorage.getItem(LAST_VAULT_KEY);
          if (stored) {
            const last = JSON.parse(stored) as LastVault;
            if (last.profile && last.path) {
              profile = last.profile;
              defaultPath = last.path;
            }
          }
        } catch {}

        // Try to open vault silently
        await invoke('open_vault', { path: defaultPath, profile });
        setVaultOpen(true);

        const [notesList, tagsList, profilesList] = await Promise.all([
          invoke<Note[]>("list_notes", { limit: 500, offset: 0 }),
          invoke<TagInfo[]>("list_tags"),
          invoke<ProfileInfo[]>("list_profiles"),
        ]);

        setNotes(notesList);
        setTags(tagsList);
        setProfiles(profilesList);

        const ap = profilesList.find((p) => p.is_active) || profilesList[0] || null;
        setActiveProfile(ap);
      } catch {
        setVaultOpen(false);
      }
    })();
  }, []);

  // ── Note operations ────────────────────────────────────────

  const selectNote = useCallback(async (note: Note) => {
    try {
      // Clear search highlight when switching notes
      setEditorSearchQuery("");
      const nc = await invoke<NoteContent>("get_note", { identifier: note.path });
      setActiveNote(nc.note);
      setNoteContent(nc.body);
      setWordCount(nc.body.split(/\s+/).filter(Boolean).length);

      // Load backlinks
      const bl = await invoke<any[]>("get_backlinks", { identifier: note.path });
      setBacklinks(bl);
    } catch (e) {
      console.error("Failed to load note:", e);
    }
  }, []);

  const createNote = useCallback(async () => {
    try {
      const note = await invoke<Note>("create_note", {
        req: { title: "Untitled Note", content: "# Untitled Note\n\n", tags: [] },
      });
      setNotes((prev) => [note, ...prev]);
      await selectNote(note);
    } catch (e) {
      console.error("Failed to create note:", e);
    }
  }, [selectNote]);

  const saveNote = useCallback(async () => {
    if (!activeNote) return;
    setSaving(true);
    try {
      await invoke("update_note", { identifier: activeNote.path, content: noteContent });
      setLastSaved(new Date());
    } catch (e) {
      console.error("Failed to save note:", e);
    } finally {
      setSaving(false);
    }
  }, [activeNote, noteContent]);

  const deleteNote = useCallback(async (note: Note) => {
    const confirmed = window.confirm(`Delete "${note.title}"? This cannot be undone.`);
    if (!confirmed) return;

    try {
      await invoke("delete_note", { identifier: note.path });
      setNotes((prev) => prev.filter((n) => n.id !== note.id));
      if (activeNote?.id === note.id) {
        setActiveNote(null);
        setNoteContent("");
      }
    } catch (e) {
      console.error("Failed to delete note:", e);
    }
  }, [activeNote]);

  // Debounced save
  const handleContentChange = useCallback((value: string) => {
    setNoteContent(value);
    setWordCount(value.split(/\s+/).filter(Boolean).length);

    if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    saveTimerRef.current = setTimeout(() => {
      if (activeNote) {
        invoke("update_note", { identifier: activeNote.path, content: value })
          .then(() => setLastSaved(new Date()))
          .catch(console.error);
      }
    }, 1000);
  }, [activeNote]);

  // ── Category & Favorites ────────────────────────────────────

  const handleSetCategory = useCallback(async (noteId: string, category: string | null) => {
    try {
      await invoke("set_note_category", { noteId, category });
      // Update local notes state
      setNotes((prev) => prev.map((n) => n.id === noteId ? { ...n, category } : n));
      if (activeNote?.id === noteId) {
        setActiveNote((prev) => prev ? { ...prev, category } : prev);
      }
    } catch (e) {
      console.error("Failed to set category:", e);
    }
  }, [activeNote]);

  const handleToggleFavorite = useCallback(async (noteId: string, favorite: boolean) => {
    try {
      await invoke("toggle_note_favorite", { noteId, favorite });
      // Update local notes state
      setNotes((prev) => prev.map((n) => n.id === noteId ? { ...n, is_favorite: favorite } : n));
      if (activeNote?.id === noteId) {
        setActiveNote((prev) => prev ? { ...prev, is_favorite: favorite } : prev);
      }
    } catch (e) {
      console.error("Failed to toggle favorite:", e);
    }
  }, [activeNote]);

  // ── Search ─────────────────────────────────────────────────

  const handleSearch = useCallback(async (query: string) => {
    if (!query.trim()) {
      setSearchResults([]);
      return;
    }
    try {
      const results = await invoke<SearchResult[]>("search_notes", { req: { query, limit: 20 } });
      setSearchResults(results);
    } catch (e) {
      console.error("Search failed:", e);
    }
  }, []);

  const handleSearchSelect = useCallback(async (path: string, query: string) => {
    try {
      const nc = await invoke<NoteContent>("get_note", { identifier: path });
      setActiveNote(nc.note);
      setNoteContent(nc.body);
      setSearchOpen(false);
      setWordCount(nc.body.split(/\s+/).filter(Boolean).length);

      // Highlight the search query in the editor after the document loads
      if (query.trim()) {
        setEditorSearchQuery(query);
      }

      // Load backlinks for the opened note
      const bl = await invoke<any[]>("get_backlinks", { identifier: path });
      setBacklinks(bl);
    } catch (e) {
      console.error("Failed to open search result:", e);
    }
  }, []);

  // ── Tags ───────────────────────────────────────────────────

  const handleSelectTag = useCallback(async (tag: string | null) => {
    setSelectedTag(tag);
    if (tag) {
      try {
        const filtered = await invoke<Note[]>("get_notes_by_tag", { tag });
        setNotes(filtered);
      } catch (e) {
        console.error("Failed to filter by tag:", e);
      }
    } else {
      try {
        const all = await invoke<Note[]>("list_notes", { limit: 500, offset: 0 });
        setNotes(all);
      } catch (e) {
        console.error("Failed to reload notes:", e);
      }
    }
  }, []);

  // ── Daily note ─────────────────────────────────────────────

  const handleDailyNote = useCallback(async () => {
    try {
      const resp = await invoke<{ path: string; content: string; exists: boolean }>("get_daily_note", {});
      if (!resp.exists) {
        const note = await invoke<Note>("create_note", {
          req: { title: `Daily/${new Date().toISOString().split("T")[0]}`, content: resp.content, folder: "Daily" },
        });
        setNotes((prev) => [note, ...prev]);
        await selectNote(note);
      } else {
        // Find the note in the list
        const found = notes.find((n) => n.path === resp.path);
        if (found) {
          await selectNote(found);
        }
      }
    } catch (e) {
      console.error("Failed to open daily note:", e);
    }
  }, [notes, selectNote]);

  // ── Profile switching ──────────────────────────────────────

  const handleSwitchProfile = useCallback(async (name: string) => {
    await loadVault(name);
  }, [loadVault]);

  // ── Reindex ────────────────────────────────────────────────

  const handleReindex = useCallback(async () => {
    try {
      const stats = await invoke<{created: number, updated: number, removed: number, errors: number}>("reindex");
      // Just reload notes and tags without reopening vault
      const [notesList, tagsList] = await Promise.all([
        invoke<Note[]>("list_notes", { limit: 500, offset: 0 }),
        invoke<TagInfo[]>("list_tags"),
      ]);
      setNotes(notesList);
      setTags(tagsList);
      console.log("Reindex complete:", stats);
    } catch (e) {
      console.error("Reindex failed:", e);
    }
  }, []);

  // ── Document import ─────────────────────────────────────────

  const handleImportComplete = useCallback(async (responses: ImportResponse[]) => {
    // Reload notes and tags to include the imported ones
    const [notesList, tagsList] = await Promise.all([
      invoke<Note[]>("list_notes", { limit: 500, offset: 0 }),
      invoke<TagInfo[]>("list_tags"),
    ]);
    setNotes(notesList);
    setTags(tagsList);

    // If only one file was imported, open it
    if (responses.length === 1) {
      const resp = responses[0];
      try {
        const nc = await invoke<NoteContent>("get_note", { identifier: resp.note_path });
        setActiveNote(nc.note);
        setNoteContent(nc.body);
        setWordCount(nc.body.split(/\s+/).filter(Boolean).length);
      } catch (e) {
        console.error("Failed to open imported note:", e);
      }
    }

    const msg = responses.length === 1
      ? `Imported: ${responses[0].title} (${responses[0].format})`
      : `Imported ${responses.length} documents`;
    setToastMessage(msg);
    setTimeout(() => setToastMessage(null), 3000);
  }, []);

  const handleImportError = useCallback((message: string) => {
    setToastMessage(`❌ ${message}`);
    setTimeout(() => setToastMessage(null), 5000);
  }, []);

  // ── Keyboard shortcuts ─────────────────────────────────────

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key === "k") {
        e.preventDefault();
        setSearchOpen(true);
      }
      if (mod && e.key === "n") {
        e.preventDefault();
        createNote();
      }
      if (mod && e.key === "s") {
        e.preventDefault();
        saveNote();
      }
      if (mod && e.key === "e") {
        e.preventDefault();
        setViewMode((v) => v === "edit" ? "split" : "edit");
      }
      if (mod && e.key === "g") {
        e.preventDefault();
        setGraphOpen(true);
      }
      if (e.key === "Escape") {
        setSearchOpen(false);
        setGraphOpen(false);
        setRightPanelOpen(false);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [createNote, saveNote]);

  // ── Render ─────────────────────────────────────────────────

  if (!vaultOpen) {
    return (
      <div className="h-screen flex flex-col">
        <WelcomeScreen onOpen={handleOpenVault} onCreate={handleCreateVault} />
        <StatusBar
          noteCount={0}
          wordCount={0}
          lastSaved={null}
          profileName="—"
          activeNoteTitle={null}
          syncStatus="offline"
        />
      </div>
    );
  }

  return (
    <div className="h-screen flex flex-col bg-[#0a0a0f]">
      {/* Top bar */}
      <div className="h-12 flex items-center justify-between px-3 border-b border-[#2a2a3a] bg-[#12121a] shrink-0">
        <div className="flex items-center gap-2">
          <button
            onClick={() => setSidebarOpen(!sidebarOpen)}
            className="p-1.5 hover:bg-[#1a1a25] rounded text-[#8888a0] hover:text-white transition-colors"
            title="Toggle sidebar"
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M3 12h18M3 6h18M3 18h18" />
            </svg>
          </button>
          <span className="text-sm font-semibold text-white ml-1">🧠 Cerebro</span>
          {activeProfile && (
            <span className="text-xs text-[#8888a0] bg-[#1a1a25] px-2 py-0.5 rounded-full ml-2">
              {activeProfile.display_name}
            </span>
          )}
        </div>

        <div className="flex items-center gap-1">
          {/* View mode toggle */}
          <div className="flex bg-[#0a0a0f] rounded-lg p-0.5 mr-2">
            {(["edit", "split", "preview"] as ViewMode[]).map((mode) => (
              <button
                key={mode}
                onClick={() => setViewMode(mode)}
                className={`px-3 py-1 text-xs rounded-md transition-colors ${
                  viewMode === mode ? "bg-[#6366f1] text-white" : "text-[#8888a0] hover:text-white"
                }`}
              >
                {mode.charAt(0).toUpperCase() + mode.slice(1)}
              </button>
            ))}
          </div>

          {/* Graph button */}
          <button
            onClick={() => setGraphOpen(true)}
            className="p-1.5 hover:bg-[#1a1a25] rounded text-[#8888a0] hover:text-white transition-colors"
            title="Knowledge Graph (Ctrl+G)"
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="12" cy="12" r="3" /><circle cx="4" cy="6" r="2" /><circle cx="20" cy="6" r="2" />
              <circle cx="4" cy="18" r="2" /><circle cx="20" cy="18" r="2" />
              <path d="M9.5 10.5L5.5 7.5M14.5 10.5L18.5 7.5M9.5 13.5L5.5 16.5M14.5 13.5L18.5 16.5" />
            </svg>
          </button>

          {/* Import documents button — rendered by ImportDropZone */}
          <ImportDropZone
            onImportComplete={handleImportComplete}
            onError={handleImportError}
            vaultOpen={vaultOpen}
          />

          {/* Backlinks toggle */}
          <button
            onClick={() => setRightPanelOpen(!rightPanelOpen)}
            className={`p-1.5 rounded transition-colors ${
              rightPanelOpen ? "bg-[#6366f1]/20 text-[#818cf8]" : "text-[#8888a0] hover:bg-[#1a1a25] hover:text-white"
            }`}
            title="Backlinks panel"
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
              <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
            </svg>
          </button>

          {/* Search button */}
          <button
            onClick={() => setSearchOpen(true)}
            className="p-1.5 hover:bg-[#1a1a25] rounded text-[#8888a0] hover:text-white transition-colors"
            title="Search (Ctrl+K)"
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="11" cy="11" r="8" /><path d="M21 21l-4.35-4.35" />
            </svg>
          </button>
        </div>
      </div>

      {/* Main content */}
      <div className="flex-1 flex overflow-hidden">
        {/* Sidebar */}
        {sidebarOpen && (
          <Sidebar
            notes={notes}
            tags={tags}
            activeNote={activeNote}
            profiles={profiles}
            activeProfile={activeProfile}
            collapsed={!sidebarOpen}
            onSelectNote={selectNote}
            onCreateNote={createNote}
            onSelectTag={handleSelectTag}
            selectedTag={selectedTag}
            onSwitchProfile={handleSwitchProfile}
            onDailyNote={handleDailyNote}
            onToggleCollapse={() => setSidebarOpen(false)}
            onReindex={handleReindex}
            searchQuery={searchQuery}
            onSearchChange={setSearchQuery}
            onSetCategory={handleSetCategory}
            onToggleFavorite={handleToggleFavorite}
            onDeleteNote={deleteNote}
          />
        )}

        {/* Editor area */}
        <div className="flex-1 flex overflow-hidden">
          {activeNote ? (
            <>
              {(viewMode === "edit" || viewMode === "split") && (
                <div className={`flex-1 overflow-hidden ${viewMode === "split" ? "border-r border-[#2a2a3a]" : ""}`}>
                  <Editor
                    value={noteContent}
                    onChange={handleContentChange}
                    onSave={saveNote}
                    searchQuery={editorSearchQuery}
                    onSearchQueryCleared={() => setEditorSearchQuery("")}
                  />
                </div>
              )}
              {(viewMode === "preview" || viewMode === "split") && (
                <div className="flex-1 overflow-hidden bg-[#0a0a0f]">
                  <MarkdownPreview content={noteContent} />
                </div>
              )}
            </>
          ) : (
            <div className="flex-1 flex items-center justify-center text-[#8888a0]">
              <div className="text-center">
                <div className="text-4xl mb-4">📝</div>
                <p className="text-lg">Select a note or create a new one</p>
                <p className="text-sm mt-2">Ctrl+N to create · Ctrl+K to search</p>
              </div>
            </div>
          )}

          {/* Right panel — Backlinks */}
          {rightPanelOpen && (
            <div className="w-72 border-l border-[#2a2a3a] bg-[#12121a] overflow-auto shrink-0 animate-fade-in">
              <div className="p-4">
                <h3 className="text-sm font-semibold text-white mb-3">Backlinks</h3>
                {backlinks.length === 0 ? (
                  <p className="text-xs text-[#8888a0]">No backlinks found</p>
                ) : (
                  <div className="space-y-2">
                    {backlinks.map((bl, i) => (
                      <button
                        key={i}
                        onClick={() => handleSearchSelect(bl.target_ref || "", "")}
                        className="w-full text-left p-2 rounded-lg bg-[#1a1a25] hover:bg-[#2a2a3a] transition-colors"
                      >
                        <p className="text-sm text-white truncate">{bl.target_ref}</p>
                        {bl.context && (
                          <p className="text-xs text-[#8888a0] mt-1 line-clamp-2">{bl.context}</p>
                        )}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Status bar */}
      <StatusBar
        noteCount={notes.length}
        wordCount={wordCount}
        lastSaved={lastSaved}
        profileName={activeProfile?.display_name || "—"}
        activeNoteTitle={activeNote?.title || null}
        syncStatus={saving ? "syncing" : "synced"}
      />

      {/* Search modal */}
      <SearchModal
        open={searchOpen}
        onClose={() => setSearchOpen(false)}
        onNavigate={handleSearchSelect}
        results={searchResults}
        onSearch={handleSearch}
      />

      {/* Graph view */}
      {graphOpen && <GraphViewPanel onClose={() => setGraphOpen(false)} />}

      {/* Toast notifications */}
      {toastMessage && (
        <div className="fixed bottom-14 right-4 z-50 animate-fade-in">
          <div className="bg-[#1e1e2e] border border-[#2a2a3a] rounded-lg px-4 py-3 shadow-xl max-w-sm">
            <p className="text-sm text-white">{toastMessage}</p>
          </div>
        </div>
      )}
    </div>
  );
}
