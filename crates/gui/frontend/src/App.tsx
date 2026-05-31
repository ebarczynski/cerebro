import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import Sidebar from "./components/Sidebar";
import Editor from "./components/Editor";
import SearchModal from "./components/SearchModal";
import StatusBar from "./components/StatusBar";

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
    import("marked").then(({ marked }) => {
      try {
        const result = marked.parse(content, { async: false }) as string;
        setHtml(result);
      } catch {
        setHtml(`<pre>${content}</pre>`);
      }
    }).catch(() => {
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
    invoke<{ nodes: { id: string; title: string; path: string }[]; edges: { source: string; target: string }[] }>("get_graph")
      .then(async (data) => {
        if (!svgRef.current || data.nodes.length === 0) return;

        const d3 = await import("d3");
        const svg = d3.select(svgRef.current);
        const width = svgRef.current.clientWidth;
        const height = svgRef.current.clientHeight;

        svg.selectAll("*").remove();

        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const nodes: any[] = data.nodes.map((n) => ({ ...n }));
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const edges: any[] = data.edges.map((e) => ({ ...e }));

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
          .attr("stroke", "#2a2a3a")
          .attr("stroke-width", 1);

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
          <button onClick={onClose} className="text-gray-400 hover:text-white text-2xl leading-none">&times;</button>
        </div>
        <svg ref={svgRef} className="flex-1 w-full" />
      </div>
    </div>
  );
}

// ── Welcome Screen ─────────────────────────────────────────────

function WelcomeScreen({ onInit }: { onInit: () => void }) {
  return (
    <div className="flex-1 flex items-center justify-center bg-[#0a0a0f]">
      <div className="text-center max-w-md">
        <div className="text-6xl mb-6">🧠</div>
        <h1 className="text-3xl font-bold text-white mb-3">Cerebro</h1>
        <p className="text-[#8888a0] mb-8 leading-relaxed">
          Your personal knowledge base. Store notes, connect ideas, and explore your knowledge graph.
        </p>
        <button
          onClick={onInit}
          className="px-6 py-3 bg-[#6366f1] hover:bg-[#818cf8] text-white rounded-lg font-medium transition-colors"
        >
          Open or Create Vault
        </button>
        <p className="text-xs text-[#8888a0] mt-4">
          Profiles: <span className="text-white">personal</span> · <span className="text-white">professional</span>
        </p>
      </div>
    </div>
  );
}

// ── Main App ───────────────────────────────────────────────────

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

  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // ── Load vault ─────────────────────────────────────────────

  const loadVault = useCallback(async (profileName?: string) => {
    try {
      const home = "/home/ed";
      const profile = profileName || "personal";
      const vaultPath = `${home}/.cerebro/profiles/${profile}`;

      const info = await invoke<{ name: string; path: string; note_count: number; tag_count: number }>("open_vault", {
        path: vaultPath,
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
    } catch (e) {
      console.error("Failed to load vault:", e);
      setVaultOpen(false);
    }
  }, []);

  // Init vault
  const handleInit = useCallback(async () => {
    try {
      const home = "/home/ed";
      await invoke("init_vault", { path: `${home}/.cerebro/profiles/personal`, profile: "personal" });
      await loadVault("personal");
    } catch (e) {
      console.error("Failed to init vault:", e);
    }
  }, [loadVault]);

  // Auto-load on mount
  useEffect(() => {
    loadVault();
  }, [loadVault]);

  // ── Note operations ────────────────────────────────────────

  const selectNote = useCallback(async (note: Note) => {
    try {
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

  const handleSearchSelect = useCallback(async (path: string) => {
    try {
      const nc = await invoke<NoteContent>("get_note", { identifier: path });
      setActiveNote(nc.note);
      setNoteContent(nc.body);
      setSearchOpen(false);
      setWordCount(nc.body.split(/\s+/).filter(Boolean).length);
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
      await invoke("reindex");
      await loadVault(activeProfile?.name);
    } catch (e) {
      console.error("Reindex failed:", e);
    }
  }, [activeProfile, loadVault]);

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
        <WelcomeScreen onInit={handleInit} />
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
                        onClick={() => handleSearchSelect(bl.target_ref || "")}
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
    </div>
  );
}
