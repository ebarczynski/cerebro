import { useState, useRef, useEffect, useCallback } from "react";

// --- Types ---

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

interface TagInfo {
  name: string;
  count: number;
}

interface ProfileInfo {
  name: string;
  display_name: string;
  is_active: boolean;
  notes_dir: string;
  kind?: string;
}

interface SidebarProps {
  notes: Note[];
  tags: TagInfo[];
  activeNote: Note | null;
  profiles: ProfileInfo[];
  activeProfile: ProfileInfo | null;
  collapsed: boolean;
  onSelectNote: (note: Note) => void | Promise<void>;
  onCreateNote: () => void;
  onSelectTag: (tag: string | null) => void;
  selectedTag: string | null;
  onSwitchProfile: (name: string) => void;
  onDailyNote: () => void;
  onToggleCollapse: () => void;
  onReindex: () => void;
  searchQuery: string;
  onSearchChange: (query: string) => void;
  onSetCategory?: (noteId: string, category: string | null) => void;
  onToggleFavorite?: (noteId: string, favorite: boolean) => void;
  onDeleteNote?: (note: Note) => void;
}

// --- Context Menu State ---

interface ContextMenuState {
  visible: boolean;
  x: number;
  y: number;
  noteId: string | null;
}

// --- Helpers ---

function truncatePath(path: string, maxLen: number = 28): string {
  if (path.length <= maxLen) return path;
  const start = path.length - maxLen + 3;
  return "..." + path.slice(start);
}

function formatDate(iso: string): string {
  try {
    const d = new Date(iso);
    const now = new Date();
    const diffMs = now.getTime() - d.getTime();
    const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));

    if (diffDays === 0) {
      return d.toLocaleTimeString(undefined, {
        hour: "2-digit",
        minute: "2-digit",
      });
    } else if (diffDays === 1) {
      return "Yesterday";
    } else if (diffDays < 7) {
      return `${diffDays}d ago`;
    } else {
      return d.toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
      });
    }
  } catch {
    return "";
  }
}

function sortNotesByUpdated(notes: Note[]): Note[] {
  return [...notes].sort((a, b) => {
    const ta = new Date(a.updated_at).getTime();
    const tb = new Date(b.updated_at).getTime();
    return tb - ta;
  });
}

function filterNotes(notes: Note[], query: string, tag: string | null, category: string | null): Note[] {
  let filtered = notes;
  if (category) {
    filtered = filtered.filter((n) => n.category === category);
  }
  if (tag) {
    filtered = filtered.filter((n) => n.tags.includes(tag));
  }
  if (query.trim()) {
    const q = query.toLowerCase();
    filtered = filtered.filter((n) => n.title.toLowerCase().includes(q));
  }
  return sortNotesByUpdated(filtered);
}

// --- Main Component ---

export default function Sidebar({
  notes,
  tags,
  activeNote,
  profiles,
  activeProfile,
  collapsed,
  onSelectNote,
  onCreateNote,
  onSelectTag,
  selectedTag,
  onSwitchProfile,
  onDailyNote,
  onToggleCollapse,
  onReindex,
  searchQuery,
  onSearchChange,
  onSetCategory,
  onToggleFavorite,
  onDeleteNote,
}: SidebarProps) {
  const [tagsExpanded, setTagsExpanded] = useState(true);
  const [profileDropdownOpen, setProfileDropdownOpen] = useState(false);
  const [activeCategory, setActiveCategory] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState>({
    visible: false,
    x: 0,
    y: 0,
    noteId: null,
  });

  const profileDropdownRef = useRef<HTMLDivElement>(null);
  const contextMenuRef = useRef<HTMLDivElement>(null);

  // Close profile dropdown on outside click
  useEffect(() => {
    function handleClick(e: MouseEvent) {
      if (
        profileDropdownRef.current &&
        !profileDropdownRef.current.contains(e.target as Node)
      ) {
        setProfileDropdownOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, []);

  // Close context menu on any click or scroll
  useEffect(() => {
    function handleClose() {
      setContextMenu((prev) => ({ ...prev, visible: false }));
    }
    if (contextMenu.visible) {
      document.addEventListener("click", handleClose);
      document.addEventListener("scroll", handleClose, true);
      return () => {
        document.removeEventListener("click", handleClose);
        document.removeEventListener("scroll", handleClose, true);
      };
    }
  }, [contextMenu.visible]);

  const handleNoteContextMenu = useCallback(
    (e: React.MouseEvent, noteId: string) => {
      e.preventDefault();
      e.stopPropagation();
      setContextMenu({ visible: true, x: e.clientX, y: e.clientY, noteId });
    },
    []
  );

  const handleDeleteNote = useCallback(() => {
    if (contextMenu.noteId && onDeleteNote) {
      const note = notes.find((n) => n.id === contextMenu.noteId);
      if (note) {
        onDeleteNote(note);
      }
    }
    setContextMenu((prev) => ({ ...prev, visible: false }));
  }, [contextMenu.noteId, notes, onDeleteNote]);

  // Category tabs — built-in defaults + any custom categories found in notes
  const builtinCategories = ["estimations", "tech decisions", "dailies"];
  const customCategories = [...new Set(
    notes.map((n) => n.category).filter((c): c is string => !!c && !builtinCategories.includes(c))
  )].sort();
  const allCategories = [...builtinCategories, ...customCategories];

  const favoriteNotes = notes.filter((n) => n.is_favorite);
  const displayedNotes = filterNotes(notes, searchQuery, selectedTag, activeCategory);

  const sidebarWidth = collapsed ? "w-14" : "w-72";

  return (
    <aside
      className={`
        ${sidebarWidth}
        flex flex-col
        bg-[#12121a]
        border-r border-[#2a2a3a]
        h-full
        transition-all duration-300 ease-in-out
        overflow-hidden
        relative
        select-none
      `}
    >
      {/* --- Collapse Toggle --- */}
      <div className="flex items-center justify-between px-2 py-2 border-b border-[#2a2a3a] min-h-[44px]">
        {!collapsed && (
          <span className="text-[#8b8ba7] text-xs font-semibold uppercase tracking-wider truncate">
            Cerebro
          </span>
        )}
        <button
          onClick={onToggleCollapse}
          className="
            p-1.5 rounded-md
            text-[#8b8ba7] hover:text-[#c8c8e0] hover:bg-[#1e1e2e]
            transition-colors duration-150
            flex-shrink-0
          "
          title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
        >
          {/* Double chevron icon */}
          {collapsed ? (
            <svg
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <polyline points="9 18 15 12 9 6" />
            </svg>
          ) : (
            <svg
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <polyline points="11 17 6 12 11 7" />
              <polyline points="18 17 13 12 18 7" />
            </svg>
          )}
        </button>
      </div>

      {/* --- Search Input --- */}
      <div className={`px-2 py-2 ${collapsed ? "flex justify-center" : ""}`}>
        {collapsed ? (
          <button
            className="
              p-1.5 rounded-md
              text-[#8b8ba7] hover:text-[#c8c8e0] hover:bg-[#1e1e2e]
              transition-colors duration-150
            "
            title="Search notes"
          >
            <svg
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <circle cx="11" cy="11" r="8" />
              <line x1="21" y1="21" x2="16.65" y2="16.65" />
            </svg>
          </button>
        ) : (
          <div className="relative">
            <svg
              className="absolute left-2.5 top-1/2 -translate-y-1/2 text-[#8b8ba7] pointer-events-none"
              width="15"
              height="15"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <circle cx="11" cy="11" r="8" />
              <line x1="21" y1="21" x2="16.65" y2="16.65" />
            </svg>
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => onSearchChange(e.target.value)}
              placeholder="Search notes..."
              className="
                w-full pl-8 pr-3 py-1.5
                bg-[#1a1a28] border border-[#2a2a3a] rounded-md
                text-sm text-[#c8c8e0] placeholder-[#55556a]
                outline-none focus:border-[#7c5cfc] focus:ring-1 focus:ring-[#7c5cfc]/40
                transition-colors duration-150
              "
            />
          </div>
        )}
      </div>

      {/* --- Quick Actions --- */}
      <div
        className={`px-2 pb-2 flex gap-1.5 ${
          collapsed ? "flex-col items-center" : ""
        }`}
      >
        {/* New Note */}
        <button
          onClick={onCreateNote}
          className={`
            flex items-center gap-1.5 rounded-md
            text-[#c8c8e0] hover:bg-[#7c5cfc]/20 hover:text-[#a78bfa]
            transition-colors duration-150
            ${collapsed ? "p-2 justify-center" : "px-3 py-1.5 text-sm"}
          `}
          title="New note"
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <line x1="12" y1="5" x2="12" y2="19" />
            <line x1="5" y1="12" x2="19" y2="12" />
          </svg>
          {!collapsed && <span>New Note</span>}
        </button>

        {/* Daily Note */}
        <button
          onClick={onDailyNote}
          className={`
            flex items-center gap-1.5 rounded-md
            text-[#c8c8e0] hover:bg-[#7c5cfc]/20 hover:text-[#a78bfa]
            transition-colors duration-150
            ${collapsed ? "p-2 justify-center" : "px-3 py-1.5 text-sm"}
          `}
          title="Daily note"
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <rect x="3" y="4" width="18" height="18" rx="2" ry="2" />
            <line x1="16" y1="2" x2="16" y2="6" />
            <line x1="8" y1="2" x2="8" y2="6" />
            <line x1="3" y1="10" x2="21" y2="10" />
          </svg>
          {!collapsed && <span>Daily Note</span>}
        </button>
      </div>

      {/* --- Category Tabs --- */}
      {!collapsed && (
        <div className="px-2 pb-1">
          <div className="flex flex-wrap gap-1">
            <button
              onClick={() => setActiveCategory(null)}
              className={`
                px-2 py-0.5 rounded-full text-[10px] transition-colors duration-150
                ${!activeCategory
                  ? "bg-[#7c5cfc] text-white"
                  : "bg-[#1e1e2e] text-[#8b8ba7] hover:bg-[#2a2a3a] hover:text-[#c8c8e0]"}
              `}
            >
              All
            </button>
            {allCategories.map((cat) => (
              <button
                key={cat}
                onClick={() => setActiveCategory(activeCategory === cat ? null : cat)}
                className={`
                  px-2 py-0.5 rounded-full text-[10px] transition-colors duration-150
                  ${activeCategory === cat
                    ? "bg-[#7c5cfc] text-white"
                    : "bg-[#1e1e2e] text-[#8b8ba7] hover:bg-[#2a2a3a] hover:text-[#c8c8e0]"}
                `}
              >
                {cat}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* --- Scrollable Middle Area --- */}
      <div className="flex-1 overflow-y-auto overflow-x-hidden thin-scrollbar">
        {/* --- Favorites Section --- */}
        {!collapsed && !activeCategory && favoriteNotes.length > 0 && (
          <div className="px-2 pb-1">
            <div className="flex items-center gap-1 px-1 py-1">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="#f59e0b" stroke="#f59e0b" strokeWidth="1">
                <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
              </svg>
              <span className="text-[11px] font-semibold uppercase tracking-wider text-[#f59e0b]">
                Favorites
              </span>
            </div>
            <div className="space-y-0.5">
              {favoriteNotes.slice(0, 5).map((note) => {
                const isActive = activeNote?.id === note.id;
                return (
                  <div
                    key={note.id}
                    onClick={() => onSelectNote(note)}
                    className={`
                      px-2.5 py-1.5 rounded-md cursor-pointer transition-colors duration-100 group
                      ${isActive
                        ? "bg-[#7c5cfc]/20 text-[#c8c8e0] border-l-2 border-[#7c5cfc]"
                        : "text-[#a0a0b8] hover:bg-[#1e1e2e] border-l-2 border-transparent"}
                    `}
                  >
                    <div className="flex items-center justify-between gap-1">
                      <span className="text-sm truncate">{note.title || "Untitled"}</span>
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          onToggleFavorite?.(note.id, false);
                        }}
                        className="opacity-0 group-hover:opacity-100 transition-opacity p-0.5"
                        title="Remove from favorites"
                      >
                        <svg width="12" height="12" viewBox="0 0 24 24" fill="#f59e0b" stroke="#f59e0b" strokeWidth="1">
                          <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
                        </svg>
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {/* --- Notes List --- */}
        {!collapsed && (
          <div className="px-2 pb-2">
            <div className="flex items-center justify-between px-1 py-1">
              <span className="text-[#8b8ba7] text-[11px] font-semibold uppercase tracking-wider">
                Notes
              </span>
              {selectedTag && (
                <button
                  onClick={() => onSelectTag(null)}
                  className="text-[11px] text-[#7c5cfc] hover:text-[#a78bfa] transition-colors"
                  title="Clear tag filter"
                >
                  Clear filter
                </button>
              )}
            </div>

            {displayedNotes.length === 0 ? (
              <div className="px-2 py-6 text-center">
                <svg
                  className="mx-auto mb-2 text-[#55556a]"
                  width="32"
                  height="32"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z" />
                  <polyline points="14 2 14 8 20 8" />
                </svg>
                <p className="text-[#55556a] text-xs">
                  {notes.length === 0
                    ? "No notes yet. Create one!"
                    : "No matching notes."}
                </p>
              </div>
            ) : (
              <div className="space-y-0.5">
                {displayedNotes.map((note) => {
                  const isActive = activeNote?.id === note.id;
                  return (
                    <div
                      key={note.id}
                      onClick={() => onSelectNote(note)}
                      onContextMenu={(e) =>
                        handleNoteContextMenu(e, note.id)
                      }
                      className={`
                        px-2.5 py-2 rounded-md cursor-pointer
                        transition-colors duration-100 group
                        ${
                          isActive
                            ? "bg-[#7c5cfc]/20 text-[#c8c8e0] border-l-2 border-[#7c5cfc]"
                            : "text-[#a0a0b8] hover:bg-[#1e1e2e] border-l-2 border-transparent"
                        }
                      `}
                    >
                      <div className="flex items-start justify-between gap-1">
                        <span
                          className={`text-sm font-medium truncate leading-tight ${
                            isActive ? "text-[#e0e0f0]" : ""
                          }`}
                        >
                          {note.title || "Untitled"}
                        </span>
                        <span className="text-[10px] text-[#55556a] whitespace-nowrap flex-shrink-0 mt-0.5">
                          {formatDate(note.updated_at)}
                        </span>
                      </div>
                      <div className="text-[11px] text-[#55556a] truncate mt-0.5">
                        {truncatePath(note.path)}
                      </div>
                      {note.tags.length > 0 && (
                        <div className="flex flex-wrap gap-1 mt-1">
                          {note.tags.slice(0, 3).map((tag) => (
                            <span
                              key={tag}
                              className="
                                inline-block px-1.5 py-0
                                text-[10px] leading-[18px] rounded
                                bg-[#7c5cfc]/15 text-[#a78bfa]
                              "
                            >
                              {tag}
                            </span>
                          ))}
                          {note.tags.length > 3 && (
                            <span className="text-[10px] text-[#55556a] leading-[18px]">
                              +{note.tags.length - 3}
                            </span>
                          )}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        )}

        {/* Collapsed: show note count indicator */}
        {collapsed && (
          <div className="flex flex-col items-center gap-1 py-2">
            <div className="w-6 h-6 flex items-center justify-center text-[#8b8ba7]">
              <svg
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z" />
                <polyline points="14 2 14 8 20 8" />
              </svg>
            </div>
            <span className="text-[9px] text-[#55556a]">
              {displayedNotes.length}
            </span>
          </div>
        )}

        {/* --- Tags Section --- */}
        {!collapsed && (
          <div className="px-2 pb-2">
            <button
              onClick={() => setTagsExpanded(!tagsExpanded)}
              className="
                flex items-center gap-1.5 w-full px-1 py-1
                text-[#8b8ba7] text-[11px] font-semibold uppercase tracking-wider
                hover:text-[#c8c8e0] transition-colors duration-150
              "
            >
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                className={`transition-transform duration-200 ${
                  tagsExpanded ? "rotate-90" : ""
                }`}
              >
                <polyline points="9 18 15 12 9 6" />
              </svg>
              Tags
            </button>

            {tagsExpanded && (
              <>
                {tags.length === 0 ? (
                  <p className="text-[#55556a] text-xs px-2 py-3 text-center">
                    No tags yet. Add tags to your notes.
                  </p>
                ) : (
                  <div className="flex flex-wrap gap-1 px-1 pt-1">
                    {tags.map((tag) => (
                      <button
                        key={tag.name}
                        onClick={() =>
                          onSelectTag(
                            selectedTag === tag.name ? null : tag.name
                          )
                        }
                        className={`
                          inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px]
                          transition-colors duration-150 cursor-pointer
                          ${
                            selectedTag === tag.name
                              ? "bg-[#7c5cfc] text-white"
                              : "bg-[#1e1e2e] text-[#8b8ba7] hover:bg-[#2a2a3a] hover:text-[#c8c8e0]"
                          }
                        `}
                      >
                        <span>{tag.name}</span>
                        <span
                          className={`text-[9px] ${
                            selectedTag === tag.name
                              ? "text-white/70"
                              : "text-[#55556a]"
                          }`}
                        >
                          {tag.count}
                        </span>
                      </button>
                    ))}
                  </div>
                )}
              </>
            )}
          </div>
        )}

        {/* Collapsed: show tag icon */}
        {collapsed && (
          <div className="flex flex-col items-center gap-1 py-2">
            <button
              onClick={() => onSelectTag(selectedTag ? null : "__cycle__")}
              className="
                w-8 h-8 flex items-center justify-center rounded-md
                text-[#8b8ba7] hover:text-[#c8c8e0] hover:bg-[#1e1e2e]
                transition-colors duration-150
              "
              title="Tags"
            >
              <svg
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M20.59 13.41l-7.17 7.17a2 2 0 0 1-2.83 0L2 12V2h10l8.59 8.59a2 2 0 0 1 0 2.82z" />
                <line x1="7" y1="7" x2="7.01" y2="7" />
              </svg>
            </button>
          </div>
        )}
      </div>

      {/* --- Bottom: Profile Switcher + Reindex --- */}
      <div className="border-t border-[#2a2a3a] px-2 py-2 flex flex-col gap-1">
        {/* Reindex Button */}
        <button
          onClick={onReindex}
          className={`
            flex items-center gap-1.5 rounded-md
            text-[#8b8ba7] hover:text-[#c8c8e0] hover:bg-[#1e1e2e]
            transition-colors duration-150
            ${collapsed ? "p-2 justify-center" : "px-3 py-1.5 text-sm"}
          `}
          title="Reindex knowledge base"
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M21.5 2v6h-6M2.5 22v-6h6M2 11.5a10 10 0 0 1 18.8-4.3M22 12.5a10 10 0 0 1-18.8 4.2" />
          </svg>
          {!collapsed && <span>Reindex</span>}
        </button>

        {/* Profile Switcher */}
        <div className="relative" ref={profileDropdownRef}>
          <button
            onClick={() => setProfileDropdownOpen(!profileDropdownOpen)}
            className={`
              flex items-center gap-1.5 rounded-md w-full
              text-[#c8c8e0] hover:bg-[#1e1e2e]
              transition-colors duration-150
              ${collapsed ? "p-2 justify-center" : "px-3 py-1.5 text-sm"}
            `}
            title={
              activeProfile
                ? `Profile: ${activeProfile.display_name}`
                : "Select profile"
            }
          >
            <svg
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
              <circle cx="12" cy="7" r="4" />
            </svg>
            {!collapsed && (
              <>
                <span className="truncate flex-1 text-left">
                  {activeProfile?.display_name ?? "Profile"}
                </span>
                <svg
                  width="12"
                  height="12"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  className={`transition-transform duration-200 ${
                    profileDropdownOpen ? "rotate-180" : ""
                  }`}
                >
                  <polyline points="6 9 12 15 18 9" />
                </svg>
              </>
            )}
          </button>

          {/* Profile Dropdown */}
          {profileDropdownOpen && !collapsed && (
            <div
              className="
                absolute bottom-full left-0 right-0 mb-1
                bg-[#1a1a28] border border-[#2a2a3a] rounded-md shadow-lg
                z-50 overflow-hidden
                max-h-60 overflow-y-auto
              "
            >
              {profiles.length === 0 ? (
                <p className="text-[#55556a] text-xs px-3 py-3 text-center">
                  No profiles available.
                </p>
              ) : (
                profiles.map((profile) => (
                  <button
                    key={profile.name}
                    onClick={() => {
                      onSwitchProfile(profile.name);
                      setProfileDropdownOpen(false);
                    }}
                    className={`
                      flex items-center gap-2 w-full px-3 py-2 text-left text-sm
                      transition-colors duration-100
                      ${
                        profile.is_active
                          ? "bg-[#7c5cfc]/15 text-[#a78bfa]"
                          : "text-[#a0a0b8] hover:bg-[#2a2a3a] hover:text-[#c8c8e0]"
                      }
                    `}
                  >
                    <span
                      className={`w-2 h-2 rounded-full flex-shrink-0 ${
                        profile.is_active ? "bg-[#7c5cfc]" : "bg-[#55556a]"
                      }`}
                    />
                    <span className="truncate flex-1 min-w-0">{profile.display_name}</span>
                    {profile.kind && (
                      <span
                        className="flex-shrink-0 text-[8.5px] font-semibold uppercase tracking-wide rounded px-1 py-px"
                        style={
                          profile.kind === 'business'
                            ? { color: '#f59e0b', backgroundColor: '#f59e0b1f', border: '1px solid #f59e0b55' }
                            : { color: '#6366f1', backgroundColor: '#6366f11f', border: '1px solid #6366f155' }
                        }
                      >
                        {profile.kind === 'business' ? '💼 Business' : '🔒 Private'}
                      </span>
                    )}
                  </button>
                ))
              )}
            </div>
          )}
        </div>
      </div>

      {/* --- Context Menu --- */}
      {contextMenu.visible && (
        <div
          ref={contextMenuRef}
          className="
            fixed z-[100]
            bg-[#1a1a28] border border-[#2a2a3a] rounded-md shadow-xl
            min-w-[180px] py-1 overflow-hidden
          "
          style={{
            top: contextMenu.y,
            left: contextMenu.x,
          }}
          onClick={(e) => e.stopPropagation()}
        >
          {/* Toggle Favorite */}
          <button
            onClick={() => {
              const note = notes.find((n) => n.id === contextMenu.noteId);
              if (note && onToggleFavorite) {
                onToggleFavorite(note.id, !note.is_favorite);
              }
              setContextMenu((prev) => ({ ...prev, visible: false }));
            }}
            className="
              flex items-center gap-2 w-full px-3 py-2 text-left text-sm
              text-[#f59e0b] hover:bg-[#f59e0b]/15
              transition-colors duration-100
            "
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" strokeWidth="1">
              <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
            </svg>
            {notes.find((n) => n.id === contextMenu.noteId)?.is_favorite ? "Remove from Favorites" : "Add to Favorites"}
          </button>

          {/* Category assignments */}
          <div className="border-t border-[#2a2a3a] py-1">
            <span className="px-3 py-1 text-[10px] text-[#55556a] uppercase tracking-wider">Category</span>
            {allCategories.map((cat) => (
              <button
                key={cat}
                onClick={() => {
                  if (contextMenu.noteId && onSetCategory) {
                    const note = notes.find((n) => n.id === contextMenu.noteId);
                    onSetCategory(contextMenu.noteId, note?.category === cat ? null : cat);
                  }
                  setContextMenu((prev) => ({ ...prev, visible: false }));
                }}
                className={`
                  flex items-center gap-2 w-full px-3 py-1.5 text-left text-xs
                  transition-colors duration-100
                  ${notes.find((n) => n.id === contextMenu.noteId)?.category === cat
                    ? "text-[#7c5cfc] bg-[#7c5cfc]/10"
                    : "text-[#a0a0b8] hover:bg-[#2a2a3a] hover:text-[#c8c8e0]"}
                `}
              >
                <span className="w-2 h-2 rounded-full bg-[#7c5cfc]/40" />
                {cat}
              </button>
            ))}
          </div>

          {/* Delete */}
          <div className="border-t border-[#2a2a3a]">
            <button
              onClick={handleDeleteNote}
              className="
                flex items-center gap-2 w-full px-3 py-2 text-left text-sm
                text-[#e06060] hover:bg-[#e06060]/15
                transition-colors duration-100
              "
            >
              <svg
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <polyline points="3 6 5 6 21 6" />
                <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                <line x1="10" y1="11" x2="10" y2="17" />
                <line x1="14" y1="11" x2="14" y2="17" />
              </svg>
              Delete Note
            </button>
          </div>
        </div>
      )}
    </aside>
  );
}
