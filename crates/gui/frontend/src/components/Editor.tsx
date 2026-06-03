import React, { useCallback, useEffect, useRef, useState } from 'react';
import CodeMirror, { ReactCodeMirrorRef } from '@uiw/react-codemirror';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';
import { oneDark } from '@codemirror/theme-one-dark';
import { EditorView, keymap, lineNumbers, highlightActiveLineGutter, highlightActiveLine, drawSelection, rectangularSelection, crosshairCursor } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { syntaxHighlighting, defaultHighlightStyle, bracketMatching, foldGutter, indentOnInput, foldKeymap } from '@codemirror/language';
import { closeBrackets, closeBracketsKeymap, autocompletion, completionKeymap } from '@codemirror/autocomplete';
import { searchKeymap, highlightSelectionMatches, search, SearchQuery, getSearchQuery, setSearchQuery, findNext, findPrevious, replaceNext, replaceAll } from '@codemirror/search';
import { lintKeymap } from '@codemirror/lint';
import { Extension, EditorState } from '@codemirror/state';
import { invoke } from "@tauri-apps/api/core";

const FONT_SIZE_STORAGE_KEY = 'cerebro-editor-font-size';
const DEFAULT_FONT_SIZE = 14;
const MIN_FONT_SIZE = 8;
const MAX_FONT_SIZE = 32;

function loadFontSize(): number {
  try {
    const stored = localStorage.getItem(FONT_SIZE_STORAGE_KEY);
    if (stored) {
      const parsed = parseInt(stored, 10);
      if (!isNaN(parsed) && parsed >= MIN_FONT_SIZE && parsed <= MAX_FONT_SIZE) {
        return parsed;
      }
    }
  } catch {}
  return DEFAULT_FONT_SIZE;
}

/**
 * NOTE: Search highlighting uses CodeMirror's built-in @codemirror/search extension.
 * The actual full-text search (SQLite FTS5 with Porter stemmer via `search_notes` Tauri
 * command) remains the primary search backend — it is optimal for this local vault use
 * case: in-process (no IPC overhead like ripgrep), integrated ranking, Porter stemming,
 * and works with Tauri's embedded SQLite. This editor-level highlighting is purely a
 * visual complement to navigate within a loaded note after selecting a search result.
 */

interface EditorProps {
  value: string;
  onChange: (val: string) => void;
  onSave: () => void;
  readOnly?: boolean;
  /** When set, highlights all matches of this query in the editor and jumps to the first match */
  searchQuery?: string;
  /** Called when the user dismisses the search highlight (Escape) */
  onSearchQueryCleared?: () => void;
}

const cerebroTheme = EditorView.theme({
  '&': {
    backgroundColor: '#0a0a0f',
    color: '#d4d4d8',
    height: '100%',
  },
  '.cm-content': {
    caretColor: '#6366f1',
    fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", Menlo, Monaco, monospace',
    padding: '8px 0',
  },
  '.cm-cursor': {
    borderLeftColor: '#6366f1',
    borderLeftWidth: '2px',
  },
  '.cm-activeLine': {
    backgroundColor: '#12121a',
  },
  '.cm-activeLineGutter': {
    backgroundColor: '#12121a',
  },
  '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': {
    backgroundColor: '#6366f140',
  },
  '.cm-gutters': {
    backgroundColor: '#0a0a0f',
    color: '#52525b',
    border: 'none',
    borderRight: '1px solid #1e1e2e',
  },
  '.cm-lineNumbers .cm-gutterElement': {
    minWidth: '3em',
    paddingRight: '8px',
  },
  '.cm-foldGutter': {
    width: '16px',
  },
  '.cm-scroller': {
    overflow: 'auto',
    lineHeight: '1.6',
  },
  '.cm-focused': {
    outline: 'none',
  },
  '.cm-matchingBracket': {
    backgroundColor: '#6366f130',
    color: '#a5b4fc',
    outline: '1px solid #6366f150',
  },
  '.cm-nonmatchingBracket': {
    backgroundColor: '#ef444430',
    color: '#fca5a5',
  },
  '.cm-searchMatch': {
    backgroundColor: '#FFE58F !important',
    color: '#000000 !important',
    outline: '2px solid #FFD700',
    borderRadius: '2px',
  },
  '.cm-searchMatch.cm-searchMatch-selected': {
    backgroundColor: '#FF9800 !important',
    color: '#000000 !important',
    outline: '2px solid #F57C00',
  },
  '.cm-panels': {
    backgroundColor: '#12121a',
    color: '#d4d4d8',
    borderBottom: '1px solid #2a2a3a',
  },
  '.cm-panels input, .cm-panels button': {
    fontFamily: 'inherit',
  },
  '.cm-tooltip': {
    backgroundColor: '#12121a',
    border: '1px solid #2a2a3a',
    color: '#d4d4d8',
  },
  '.cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected]': {
    backgroundColor: '#6366f130',
    color: '#e0e7ff',
  },
}, { dark: true });

const cerebroHighlightStyle = syntaxHighlighting(defaultHighlightStyle, { fallback: true });

function useDebounce<T extends (...args: any[]) => void>(callback: T, delay: number): T {
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const callbackRef = useRef(callback);

  useEffect(() => {
    callbackRef.current = callback;
  }, [callback]);

  useEffect(() => {
    return () => {
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }
    };
  }, []);

  return useCallback(
    ((...args: unknown[]) => {
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }
      timeoutRef.current = setTimeout(() => {
        callbackRef.current(...args);
      }, delay);
    }) as T,
    [delay]
  );
}

const Editor: React.FC<EditorProps> = ({ value, onChange, onSave, readOnly = false, searchQuery, onSearchQueryCleared }) => {
  const cmRef = useRef<ReactCodeMirrorRef>(null);
  const localValueRef = useRef<string>(value);
  const isExternalUpdate = useRef<boolean>(false);
  const [matchInfo, setMatchInfo] = useState<{ current: number; total: number } | null>(null);
  const searchQueryRef = useRef<string | undefined>(searchQuery);
  const [fontSize, setFontSize] = useState<number>(loadFontSize);
  const [fontSizeToast, setFontSizeToast] = useState<string | null>(null);
  const fontSizeToastTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // ── Replace bar state ─────────────────────────────────────
  const [replaceOpen, setReplaceOpen] = useState(false);
  const [replaceText, setReplaceText] = useState('');
  const [caseSensitive, setCaseSensitiveState] = useState(false);
  const [useRegex, setUseRegex] = useState(false);

  const debouncedOnChange = useDebounce((val: string) => {
    onChange(val);
  }, 300);

  useEffect(() => {
    if (value !== localValueRef.current) {
      isExternalUpdate.current = true;
      localValueRef.current = value;
    }
  }, [value]);

  const handleChange = useCallback(
    (val: string) => {
      localValueRef.current = val;
      debouncedOnChange(val);
    },
    [debouncedOnChange]
  );

  // ── Font size adjustment with toast feedback ──────────────
  const showFontSizeToast = useCallback((size: number) => {
    if (fontSizeToastTimer.current) {
      clearTimeout(fontSizeToastTimer.current);
    }
    setFontSizeToast(`${size}px`);
    fontSizeToastTimer.current = setTimeout(() => setFontSizeToast(null), 1500);
  }, []);

  const adjustFontSize = useCallback((delta: number) => {
    setFontSize((prev) => {
      const next = Math.max(MIN_FONT_SIZE, Math.min(MAX_FONT_SIZE, prev + delta));
      if (next !== prev) {
        try {
          localStorage.setItem(FONT_SIZE_STORAGE_KEY, String(next));
        } catch {}
        showFontSizeToast(next);
      }
      return next;
    });
    return true;
  }, [showFontSizeToast]);

  const resetFontSize = useCallback(() => {
    setFontSize(DEFAULT_FONT_SIZE);
    try {
      localStorage.setItem(FONT_SIZE_STORAGE_KEY, String(DEFAULT_FONT_SIZE));
    } catch {}
    showFontSizeToast(DEFAULT_FONT_SIZE);
    return true;
  }, [showFontSizeToast]);

  // Font size keyboard shortcuts keymap
  const fontSizeKeymap = keymap.of([
    {
      key: 'Mod-=',
      run: () => adjustFontSize(1),
      preventDefault: true,
    },
    {
      key: 'Mod-Plus',
      run: () => adjustFontSize(1),
      preventDefault: true,
    },
    {
      key: 'Mod-Minus',
      run: () => adjustFontSize(-1),
      preventDefault: true,
    },
    {
      key: 'Mod-0',
      run: () => resetFontSize(),
      preventDefault: true,
    },
  ]);

  // Dynamic font size theme — must target CodeMirror internals directly
  const fontSizeTheme = EditorView.theme({
    '.cm-content': {
      fontSize: `${fontSize}px`,
    },
    '.cm-line': {
      fontSize: `${fontSize}px`,
    },
    '.cm-gutters': {
      fontSize: `${fontSize}px`,
    },
    '.cm-tooltip': {
      fontSize: `${fontSize}px`,
    },
  });

  // ── Search query highlighting ─────────────────────────────

  // Keep ref in sync for use in update listener
  searchQueryRef.current = searchQuery;

  // Count matches and determine current match position relative to the cursor
  const updateMatchInfo = useCallback((view: EditorView) => {
    const query = getSearchQuery(view.state);
    if (!query.search || !query.valid) {
      setMatchInfo(null);
      return;
    }
    const matches: { from: number; to: number }[] = [];
    const cursor = query.getCursor(view.state);
    let iterResult = cursor.next();
    while (!iterResult.done) {
      matches.push({ from: iterResult.value.from, to: iterResult.value.to });
      iterResult = cursor.next();
    }
    const total = matches.length;
    if (total === 0) {
      setMatchInfo(null);
      return;
    }
    // Find the match closest after the main selection head
    const head = view.state.selection.main.head;
    let current = 1;
    for (let i = 0; i < matches.length; i++) {
      if (matches[i].from >= head) {
        current = i + 1;
        break;
      }
      if (i === matches.length - 1) {
        current = 1; // wrap around
      }
    }
    setMatchInfo({ current, total });
  }, []);

  // Apply search query when it changes (set from parent via SearchModal selection)
  useEffect(() => {
    const view = cmRef.current?.view;
    if (!view) return;

    if (searchQuery && searchQuery.trim()) {
      const sq = new SearchQuery({
        search: searchQuery,
        caseSensitive,
        regexp: useRegex,
      });
      view.dispatch({
        effects: setSearchQuery.of(sq),
      });
      // Focus the editor and jump to first match
      view.focus();
      // Move cursor to start so findNext goes to the very first match
      view.dispatch({ selection: { anchor: 0 } });
      findNext(view);
      updateMatchInfo(view);
    } else {
      // Clear search when query is cleared / note switches
      const current = getSearchQuery(view.state);
      if (current.search) {
        view.dispatch({
          effects: setSearchQuery.of(new SearchQuery({ search: '' })),
        });
      }
      setMatchInfo(null);
    }
  }, [searchQuery, updateMatchInfo, caseSensitive, useRegex]);

  // Update match info on editor content changes or selection changes
  const searchUpdateListener = EditorView.updateListener.of((update) => {
    if (update.docChanged || update.selectionSet) {
      const query = getSearchQuery(update.state);
      if (query.search && query.valid) {
        updateMatchInfo(update.view);
      }
    }
  });

  const saveKeymap = keymap.of([
    {
      key: 'Mod-s',
      run: () => {
        onSave();
        return true;
      },
      preventDefault: true,
    },
  ]);

  // Search navigation keymap: F3/Shift+F3 + Escape to clear + Enter to cycle + Ctrl+H for replace
  const searchNavKeymap = keymap.of([
    {
      key: 'F3',
      run: (view) => {
        findNext(view);
        updateMatchInfo(view);
        return true;
      },
      preventDefault: true,
    },
    {
      key: 'Shift-F3',
      run: (view) => {
        findPrevious(view);
        updateMatchInfo(view);
        return true;
      },
      preventDefault: true,
    },
    {
      key: 'Mod-h',
      run: (view) => {
        setReplaceOpen((prev) => !prev);
        return true;
      },
      preventDefault: true,
    },
    {
      key: 'Mod-f',
      run: (view) => {
        setReplaceOpen(true);
        return false; // let default search panel also handle it
      },
    },
    {
      key: 'Escape',
      run: (view) => {
        if (replaceOpen) {
          setReplaceOpen(false);
          return true;
        }
        const q = getSearchQuery(view.state);
        if (q.search) {
          view.dispatch({
            effects: setSearchQuery.of(new SearchQuery({ search: '' })),
          });
          setMatchInfo(null);
          onSearchQueryCleared?.();
          return true;
        }
        return false;
      },
    },
    {
      key: 'Enter',
      run: (view) => {
        const q = getSearchQuery(view.state);
        if (q.search) {
          findNext(view);
          updateMatchInfo(view);
          return true;
        }
        return false;
      },
    },
  ]);

  // Image paste and drag-drop handler
  const imageHandlers = EditorView.domEventHandlers({
    paste(event) {
      const items = event.clipboardData?.items;
      if (!items) return false;

      for (const item of items) {
        if (item.type.startsWith('image/')) {
          event.preventDefault();
          const file = item.getAsFile();
          if (!file) continue;

          const reader = new FileReader();
          reader.onload = async () => {
            const base64 = (reader.result as string).split(',')[1];
            try {
              const imagePath = await invoke<string>("save_image", {
                fileName: file.name || `paste_${Date.now()}.png`,
                data: base64,
              });
              const view = cmRef.current?.view;
              if (view) {
                const cursor = view.state.selection.main.head;
                view.dispatch({
                  changes: { from: cursor, insert: `\n![image](${imagePath})\n` }
                });
              }
            } catch (e) {
              console.error("Failed to save pasted image:", e);
            }
          };
          reader.readAsDataURL(file);
          return true;
        }
      }
      return false;
    },
    drop(event) {
      const files = event.dataTransfer?.files;
      if (!files || files.length === 0) return false;

      for (const file of files) {
        if (file.type.startsWith('image/')) {
          event.preventDefault();
          const reader = new FileReader();
          reader.onload = async () => {
            const base64 = (reader.result as string).split(',')[1];
            try {
              const imagePath = await invoke<string>("save_image", {
                fileName: file.name,
                data: base64,
              });
              const view = cmRef.current?.view;
              if (view) {
                const cursor = view.state.selection.main.head;
                view.dispatch({
                  changes: { from: cursor, insert: `\n![${file.name}](${imagePath})\n` }
                });
              }
            } catch (e) {
              console.error("Failed to save dropped image:", e);
            }
          };
          reader.readAsDataURL(file);
          return true;
        }
      }
      return false;
    },
  });

  const extensions: Extension[] = [
    cerebroTheme,
    fontSizeTheme,
    cerebroHighlightStyle,
    lineNumbers(),
    highlightActiveLineGutter(),
    history(),
    foldGutter(),
    highlightActiveLine(),
    drawSelection(),
    rectangularSelection(),
    crosshairCursor(),
    EditorState.allowMultipleSelections.of(true),
    indentOnInput(),
    bracketMatching(),
    closeBrackets(),
    autocompletion(),
    highlightSelectionMatches(),
    search(),
    searchUpdateListener,
    EditorView.lineWrapping,
    markdown({ base: markdownLanguage, codeLanguages: languages }),
    keymap.of([
      ...closeBracketsKeymap,
      ...defaultKeymap,
      ...searchKeymap,
      ...historyKeymap,
      ...foldKeymap,
      ...completionKeymap,
      ...lintKeymap,
      indentWithTab,
    ]),
    saveKeymap,
    fontSizeKeymap,
    searchNavKeymap,
    imageHandlers,
  ];

  if (readOnly) {
    extensions.push(EditorState.readOnly.of(true));
  }

  // ── Replace handlers ──────────────────────────────────────
  const handleReplaceNext = useCallback(() => {
    const view = cmRef.current?.view;
    if (!view) return;
    // Update the search query with the replace field, then call replaceNext
    const current = getSearchQuery(view.state);
    const updated = new SearchQuery({
      search: current.search,
      caseSensitive,
      regexp: useRegex,
      replace: replaceText,
    });
    view.dispatch({ effects: setSearchQuery.of(updated) });
    replaceNext(view);
    findNext(view);
    updateMatchInfo(view);
  }, [replaceText, caseSensitive, useRegex, updateMatchInfo]);

  const handleReplaceAll = useCallback(() => {
    const view = cmRef.current?.view;
    if (!view) return;
    const confirmed = window.confirm(`Replace all occurrences with "${replaceText}"?`);
    if (!confirmed) return;
    const current = getSearchQuery(view.state);
    const updated = new SearchQuery({
      search: current.search,
      caseSensitive,
      regexp: useRegex,
      replace: replaceText,
    });
    view.dispatch({ effects: setSearchQuery.of(updated) });
    replaceAll(view);
    updateMatchInfo(view);
  }, [replaceText, caseSensitive, useRegex, updateMatchInfo]);

  const toggleCaseSensitive = useCallback(() => {
    setCaseSensitiveState((prev) => !prev);
  }, []);

  const toggleRegex = useCallback(() => {
    setUseRegex((prev) => !prev);
  }, []);

  return (
    <div className="h-full w-full overflow-hidden relative flex flex-col" style={{ backgroundColor: '#0a0a0f' }}>
      {/* Replace bar */}
      {replaceOpen && (
        <div
          className="flex items-center gap-2 px-3 py-2 border-b"
          style={{ backgroundColor: '#12121a', borderColor: '#2a2a3a', zIndex: 40 }}
        >
          <input
            type="text"
            value={replaceText}
            onChange={(e) => setReplaceText(e.target.value)}
            placeholder="Replace with..."
            className="flex-1 bg-[#0a0a0f] text-sm text-zinc-200 placeholder-zinc-600 outline-none border border-[#2a2a3a] rounded px-2 py-1 focus:border-[#6366f1]"
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                handleReplaceNext();
              }
            }}
          />
          <button
            onClick={toggleCaseSensitive}
            className={`px-2 py-1 text-xs rounded border transition-colors ${
              caseSensitive
                ? 'bg-[#6366f1]/20 text-[#a5b4fc] border-[#6366f1]/50'
                : 'text-zinc-500 border-[#2a2a3a] hover:text-zinc-300'
            }`}
            title="Case sensitive (Alt+C)"
          >
            Aa
          </button>
          <button
            onClick={toggleRegex}
            className={`px-2 py-1 text-xs rounded border transition-colors ${
              useRegex
                ? 'bg-[#6366f1]/20 text-[#a5b4fc] border-[#6366f1]/50'
                : 'text-zinc-500 border-[#2a2a3a] hover:text-zinc-300'
            }`}
            title="Regular expression (Alt+R)"
          >
            .*
          </button>
          <button
            onClick={handleReplaceNext}
            className="px-3 py-1 text-xs rounded bg-[#6366f1]/20 text-[#a5b4fc] border border-[#6366f1]/50 hover:bg-[#6366f1]/30 transition-colors"
            title="Replace next (Enter)"
          >
            Replace
          </button>
          <button
            onClick={handleReplaceAll}
            className="px-3 py-1 text-xs rounded bg-[#6366f1]/20 text-[#a5b4fc] border border-[#6366f1]/50 hover:bg-[#6366f1]/30 transition-colors"
            title="Replace all"
          >
            All
          </button>
          <button
            onClick={() => setReplaceOpen(false)}
            className="px-1.5 py-1 text-xs rounded text-zinc-500 hover:text-zinc-300 transition-colors"
            title="Close (Escape)"
          >
            ✕
          </button>
        </div>
      )}
      <div className="flex-1 overflow-hidden">
        <CodeMirror
          ref={cmRef}
          value={value}
          onChange={handleChange}
          extensions={extensions}
          theme={oneDark}
          basicSetup={false}
          className="h-full w-full"
          style={{
            height: '100%',
          }}
        />
      </div>
      {/* Font size toast overlay */}
      {fontSizeToast && (
        <div
          style={{
            position: 'absolute',
            bottom: 16,
            right: 16,
            backgroundColor: '#1e1e2e',
            border: '1px solid #2a2a3a',
            borderRadius: 8,
            padding: '6px 14px',
            boxShadow: '0 4px 12px rgba(0,0,0,0.4)',
            zIndex: 50,
            pointerEvents: 'none',
            animation: 'fadeIn 0.15s ease-out',
          }}
        >
          <span style={{ color: '#a5b4fc', fontSize: 13, fontFamily: 'monospace' }}>
            {fontSizeToast}
          </span>
        </div>
      )}
      {/* Search match info overlay */}
      {matchInfo && (
        <div
          style={{
            position: 'absolute',
            top: 8,
            right: 8,
            backgroundColor: '#1e1e2e',
            border: '1px solid #2a2a3a',
            borderRadius: 6,
            padding: '3px 10px',
            zIndex: 50,
            pointerEvents: 'none',
            display: 'flex',
            alignItems: 'center',
            gap: 8,
          }}
        >
          <span style={{ color: '#a5b4fc', fontSize: 12, fontFamily: 'monospace' }}>
            {matchInfo.current}/{matchInfo.total} matches
          </span>
          <span style={{ color: '#52525b', fontSize: 10 }}>
            F3↓ ↑Shift+F3 Ctrl+HReplace Esc✕
          </span>
        </div>
      )}
    </div>
  );
};

export default Editor;
