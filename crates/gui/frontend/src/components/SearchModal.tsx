import React, { useState, useEffect, useRef, useCallback } from 'react';

export interface SearchResult {
  title: string;
  path: string;
  rank: number;
  snippet: string;
  tags: string[];
}

interface SearchModalProps {
  open: boolean;
  onClose: () => void;
  onNavigate: (path: string) => void;
  results: SearchResult[];
  onSearch: (query: string) => void;
}

interface RecentFile {
  title: string;
  path: string;
}

const STORAGE_KEY = 'cerebro-recent-files';
const MAX_RECENT = 8;

function getRecentFiles(): RecentFile[] {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      return JSON.parse(stored) as RecentFile[];
    }
  } catch {
    // ignore parse errors
  }
  return [];
}

function addRecentFile(title: string, path: string): void {
  try {
    const recent = getRecentFiles().filter((f) => f.path !== path);
    recent.unshift({ title, path });
    localStorage.setItem(STORAGE_KEY, JSON.stringify(recent.slice(0, MAX_RECENT)));
  } catch {
    // ignore storage errors
  }
}

function highlightMatches(text: string, query: string): React.ReactNode {
  if (!query.trim()) {
    return <span>{text}</span>;
  }

  const escapedQuery = query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const parts = text.split(new RegExp(`(${escapedQuery})`, 'gi'));

  return (
    <span>
      {parts.map((part, index) => {
        if (part.toLowerCase() === query.toLowerCase()) {
          return (
            <mark
              key={index}
              className="bg-indigo-500/30 text-indigo-200 rounded-sm px-px"
            >
              {part}
            </mark>
          );
        }
        return <span key={index}>{part}</span>;
      })}
    </span>
  );
}

function formatPath(path: string): string {
  return path.replace(/\.md$/, '').replace(/\//g, ' / ');
}

function formatTags(tags: string[]): React.ReactNode {
  if (!tags || tags.length === 0) return null;
  return (
    <span className="flex items-center gap-1 flex-shrink-0">
      {tags.slice(0, 3).map((tag) => (
        <span
          key={tag}
          className="inline-flex items-center px-1.5 py-px text-[10px] rounded bg-indigo-500/15 text-indigo-400 border border-indigo-500/20"
        >
          {tag}
        </span>
      ))}
      {tags.length > 3 && (
        <span className="text-[10px] text-zinc-600">+{tags.length - 3}</span>
      )}
    </span>
  );
}

const SearchModal: React.FC<SearchModalProps> = ({
  open,
  onClose,
  onNavigate,
  results,
  onSearch,
}) => {
  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [lastSearchedQuery, setLastSearchedQuery] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);
  const resultsContainerRef = useRef<HTMLDivElement>(null);
  const recentFiles = getRecentFiles();

  const displayItems = query.trim() ? results : recentFiles;

  // Whether the displayed results match the current query
  const resultsAreStale = query.trim() !== '' && query.trim() !== lastSearchedQuery;

  useEffect(() => {
    if (open) {
      setQuery('');
      setLastSearchedQuery('');
      setSelectedIndex(0);
      // Slight delay to ensure the DOM is ready before focusing
      requestAnimationFrame(() => {
        inputRef.current?.focus();
      });
    }
  }, [open]);

  useEffect(() => {
    setSelectedIndex(0);
  }, [results]);

  const handleSelect = useCallback(
    (item: SearchResult | RecentFile) => {
      if ('rank' in item) {
        addRecentFile(item.title, item.path);
      }
      onNavigate(item.path);
      onClose();
    },
    [onNavigate, onClose]
  );

  const executeSearch = useCallback(() => {
    if (query.trim()) {
      setLastSearchedQuery(query.trim());
      onSearch(query);
    }
  }, [query, onSearch]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case 'ArrowDown': {
          e.preventDefault();
          setSelectedIndex((prev) =>
            prev < displayItems.length - 1 ? prev + 1 : 0
          );
          break;
        }
        case 'ArrowUp': {
          e.preventDefault();
          setSelectedIndex((prev) =>
            prev > 0 ? prev - 1 : displayItems.length - 1
          );
          break;
        }
        case 'Enter': {
          e.preventDefault();
          if (resultsAreStale) {
            // Query changed since last search — fire search first
            executeSearch();
          } else if (displayItems.length > 0 && selectedIndex < displayItems.length) {
            handleSelect(displayItems[selectedIndex]);
          } else if (query.trim() && results.length === 0) {
            // Fresh query, never searched yet
            executeSearch();
          }
          break;
        }
        case 'Escape': {
          e.preventDefault();
          onClose();
          break;
        }
      }
    },
    [displayItems, selectedIndex, handleSelect, onClose, resultsAreStale, executeSearch, query, results.length]
  );

  // Scroll selected item into view
  useEffect(() => {
    if (resultsContainerRef.current) {
      const selectedElement = resultsContainerRef.current.querySelector(
        `[data-index="${selectedIndex}"]`
      );
      if (selectedElement) {
        selectedElement.scrollIntoView({ block: 'nearest' });
      }
    }
  }, [selectedIndex]);

  const handleQueryChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setQuery(e.target.value);
  };

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) {
      onClose();
    }
  };

  if (!open) return null;

  const renderResultItem = (
    item: SearchResult | RecentFile,
    index: number
  ) => {
    const isSelected = index === selectedIndex;
    const isSearchResult = 'rank' in item;
    const snippet = isSearchResult ? (item as SearchResult).snippet : undefined;
    const tags = isSearchResult ? (item as SearchResult).tags : undefined;

    return (
      <button
        key={item.path}
        data-index={index}
        onClick={() => handleSelect(item)}
        onMouseEnter={() => setSelectedIndex(index)}
        className={`w-full text-left px-4 py-2.5 flex items-start gap-3 transition-colors duration-75 border-l-2 ${
          isSelected
            ? 'bg-indigo-500/10 border-l-indigo-500 text-zinc-100'
            : 'bg-transparent border-l-transparent text-zinc-400 hover:bg-zinc-800/50 hover:border-l-zinc-600'
        }`}
      >
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <svg
              className={`w-3.5 h-3.5 flex-shrink-0 ${
                isSelected ? 'text-indigo-400' : 'text-zinc-600'
              }`}
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
              />
            </svg>
            <span className="font-medium text-sm truncate">
              {isSearchResult
                ? highlightMatches(item.title, query)
                : item.title}
            </span>
            {tags && formatTags(tags)}
          </div>
          <div className="flex items-center gap-2 mt-0.5">
            <span className="text-[11px] text-zinc-600 truncate">
              {formatPath(item.path)}
            </span>
          </div>
          {snippet && (
            <p className="text-xs text-zinc-500 mt-1 line-clamp-2 leading-relaxed">
              {highlightMatches(snippet, query)}
            </p>
          )}
        </div>
        {isSearchResult && (
          <span className="text-[10px] text-zinc-700 flex-shrink-0 mt-0.5">
            {(item as SearchResult).rank.toFixed(2)}
          </span>
        )}
      </button>
    );
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center pt-[15vh] animate-in fade-in duration-150"
      onClick={handleBackdropClick}
      style={{ backgroundColor: 'rgba(0, 0, 0, 0.6)', backdropFilter: 'blur(4px)' }}
    >
      <div
        className="w-full max-w-xl rounded-lg border border-zinc-800 shadow-2xl overflow-hidden animate-in slide-in-from-top-2 duration-150"
        style={{ backgroundColor: '#12121a' }}
      >
        {/* Search Input */}
        <div className="flex items-center gap-3 px-4 py-3 border-b border-zinc-800/80">
          <svg
            className="w-4 h-4 text-zinc-500 flex-shrink-0"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"
            />
          </svg>
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={handleQueryChange}
            onKeyDown={handleKeyDown}
            placeholder="Type a search phrase, then press Enter..."
            className="flex-1 bg-transparent text-sm text-zinc-200 placeholder-zinc-600 outline-none"
            spellCheck={false}
          />
          <kbd className="hidden sm:inline-flex items-center px-1.5 py-0.5 text-[10px] text-zinc-600 bg-zinc-800/50 rounded border border-zinc-700/50">
            ESC
          </kbd>
        </div>

        {/* Results */}
        <div
          ref={resultsContainerRef}
          className="max-h-72 overflow-y-auto overscroll-contain"
        >
          {displayItems.length > 0 ? (
            <div className="py-1">
              {!query.trim() && recentFiles.length > 0 && (
                <div className="px-4 py-1.5 text-[10px] font-medium uppercase tracking-wider text-zinc-600">
                  Recent
                </div>
              )}
              {displayItems.map((item, index) => renderResultItem(item, index))}
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center py-12 px-4">
              <svg
                className="w-8 h-8 text-zinc-700 mb-2"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.5}
                  d="M9.172 16.172a4 4 0 015.656 0M9 10h.01M15 10h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                />
              </svg>
              <p className="text-xs text-zinc-600">
                {resultsAreStale
                  ? 'Press Enter to search'
                  : query.trim()
                    ? 'No results found'
                    : 'No recent files'}
              </p>
              {query.trim() && !resultsAreStale && (
                <p className="text-[11px] text-zinc-700 mt-1">
                  Try a different search term
                </p>
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between px-4 py-2 border-t border-zinc-800/60 bg-zinc-900/30">
          <div className="flex items-center gap-3 text-[10px] text-zinc-600">
            <span className="flex items-center gap-1">
              <kbd className="px-1 py-0.5 bg-zinc-800/50 rounded border border-zinc-700/50 text-[9px]">↑↓</kbd>
              navigate
            </span>
            <span className="flex items-center gap-1">
              <kbd className="px-1 py-0.5 bg-zinc-800/50 rounded border border-zinc-700/50 text-[9px]">↵</kbd>
              search / open
            </span>
            <span className="flex items-center gap-1">
              <kbd className="px-1 py-0.5 bg-zinc-800/50 rounded border border-zinc-700/50 text-[9px]">esc</kbd>
              close
            </span>
          </div>
          <span className="text-[10px] text-zinc-700">
            {displayItems.length > 0 ? `${displayItems.length} result${displayItems.length !== 1 ? 's' : ''}` : ''}
          </span>
        </div>
      </div>
    </div>
  );
};

export default SearchModal;
