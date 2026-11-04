import React, { useEffect, useRef, useState } from 'react';

export type VaultKindTag = 'private' | 'business';

export interface RecentVault {
  path: string;
  profile: string;
  name: string;
  kind: VaultKindTag;
  lastOpenedAt: number;
}

interface StatusBarProps {
  noteCount: number;
  wordCount: number;
  lastSaved: Date | null;
  profileName: string;
  activeNoteTitle: string | null;
  syncStatus: string;
  vaultPath: string | null;
  vaultKind: VaultKindTag;
  recentVaults: RecentVault[];
  onSwitchVault: (entry: RecentVault) => void;
  onOpenOther: () => void;
  onSetKind: (kind: VaultKindTag) => void;
}

// ── Vault kind theming ─────────────────────────────────────────
// Private = indigo, Business = amber — consistent everywhere the tag shows.

export const VAULT_KIND_META: Record<VaultKindTag, { label: string; icon: string; color: string }> = {
  private: { label: 'Private', icon: '🔒', color: '#6366f1' },
  business: { label: 'Business', icon: '💼', color: '#f59e0b' },
};

export function KindBadge({ kind, size }: { kind: VaultKindTag; size: 'sm' | 'lg' }) {
  const meta = VAULT_KIND_META[kind];
  const fontSize = size === 'lg' ? '10px' : '9px';
  return (
    <span
      className="inline-flex items-center gap-0.5 rounded px-1.5 flex-shrink-0"
      style={{
        fontSize,
        lineHeight: '14px',
        height: '16px',
        fontWeight: 600,
        letterSpacing: '0.03em',
        color: meta.color,
        backgroundColor: `${meta.color}1f`,
        border: `1px solid ${meta.color}55`,
        textTransform: 'uppercase',
      }}
    >
      <span style={{ fontSize: '9px', textTransform: 'none' }}>{meta.icon}</span>
      {meta.label}
    </span>
  );
}

export function formatRelativeTime(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 0) return 'just now';
  const seconds = Math.floor(diff / 1000);
  if (seconds < 60) return 'just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(ts).toLocaleDateString();
}

type SyncState = 'synced' | 'syncing' | 'error' | 'offline' | 'idle';

function getSyncState(status: string): SyncState {
  const normalized = status.toLowerCase().trim();
  if (normalized === 'synced' || normalized === 'up-to-date' || normalized === 'done') return 'synced';
  if (normalized === 'syncing' || normalized === 'in-progress' || normalized === 'uploading' || normalized === 'downloading') return 'syncing';
  if (normalized === 'error' || normalized === 'failed' || normalized === 'conflict') return 'error';
  if (normalized === 'offline' || normalized === 'disconnected') return 'offline';
  return 'idle';
}

function getSyncDotColor(state: SyncState): string {
  switch (state) {
    case 'synced':
      return '#22c55e';
    case 'syncing':
      return '#eab308';
    case 'error':
      return '#ef4444';
    case 'offline':
      return '#71717a';
    default:
      return '#52525b';
  }
}

function getSyncLabel(state: SyncState): string {
  switch (state) {
    case 'synced':
      return 'Synced';
    case 'syncing':
      return 'Syncing...';
    case 'error':
      return 'Sync Error';
    case 'offline':
      return 'Offline';
    default:
      return state;
  }
}

function formatRelativeSaved(date: Date | null): string {
  if (!date) return '';

  const now = Date.now();
  const diff = now - date.getTime();

  if (diff < 0) return 'just now';

  const seconds = Math.floor(diff / 1000);
  if (seconds < 5) return 'just now';
  if (seconds < 60) return `${seconds}s ago`;

  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;

  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;

  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;

  return date.toLocaleDateString();
}

function formatWordCount(count: number): string {
  if (count === 1) return '1 word';
  if (count < 1000) return `${count} words`;
  if (count < 1_000_000) {
    return `${(count / 1000).toFixed(1).replace(/\.0$/, '')}K words`;
  }
  return `${(count / 1_000_000).toFixed(1)}M words`;
}

function formatNoteCount(count: number): string {
  if (count === 1) return '1 note';
  return `${count} notes`;
}

/** Shorten home dir to ~ for display. */
function displayPath(p: string, home: string | null): string {
  if (home && p.startsWith(home)) return `~${p.slice(home.length)}`;
  return p;
}

const StatusBar: React.FC<StatusBarProps> = ({
  noteCount,
  wordCount,
  lastSaved,
  profileName,
  activeNoteTitle,
  syncStatus,
  vaultPath,
  vaultKind,
  recentVaults,
  onSwitchVault,
  onOpenOther,
  onSetKind,
}) => {
  const syncState = getSyncState(syncStatus);
  const dotColor = getSyncDotColor(syncState);
  const syncLabel = getSyncLabel(syncState);
  const relativeTime = formatRelativeSaved(lastSaved);

  const [vaultMenuOpen, setVaultMenuOpen] = useState(false);
  const [homeDirPath, setHomeDirPath] = useState<string | null>(null);
  const vaultMenuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    import('@tauri-apps/api/path')
      .then((m) => m.homeDir())
      .then((h) => { if (!cancelled) setHomeDirPath(h); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, []);

  // Close the vault menu on outside click
  useEffect(() => {
    if (!vaultMenuOpen) return;
    const handler = (e: MouseEvent) => {
      if (vaultMenuRef.current && !vaultMenuRef.current.contains(e.target as Node)) {
        setVaultMenuOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [vaultMenuOpen]);

  const kindMeta = VAULT_KIND_META[vaultKind];

  return (
    <div
      className="flex items-center justify-between px-3 select-none"
      style={{
        height: '24px',
        minHeight: '24px',
        backgroundColor: '#0a0a0f',
        borderTop: '1px solid #2a2a3a',
        fontSize: '11px',
        lineHeight: '24px',
        color: '#71717a',
        userSelect: 'none',
        zIndex: 50,
      }}
    >
      {/* Left side */}
      <div className="flex items-center gap-3 min-w-0">
        {/* Vault switcher — kind badge + path (click to switch between recent vaults) */}
        <div className="relative flex-shrink min-w-0" ref={vaultMenuRef}>
          <button
            onClick={() => setVaultMenuOpen(!vaultMenuOpen)}
            title={vaultPath || 'No vault open'}
            className="flex items-center gap-1.5 min-w-0 rounded px-1 -mx-1 hover:bg-[#1a1a25] transition-colors"
            style={{ height: '20px', maxWidth: '45vw' }}
          >
            <KindBadge kind={vaultKind} size="lg" />
            <span
              className="truncate min-w-0"
              style={{ fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace', color: '#a1a1b5', fontSize: '10px' }}
            >
              {vaultPath ? displayPath(vaultPath, homeDirPath) : 'no vault open'}
            </span>
            <span style={{ fontSize: '8px', color: '#71717a' }}>▲</span>
          </button>

          {vaultMenuOpen && (
            <div
              className="absolute bottom-full left-0 mb-1 rounded-lg border border-[#2a2a3a] bg-[#12121a] shadow-2xl overflow-hidden animate-fade-in"
              style={{ minWidth: '340px', zIndex: 100 }}
            >
              <div
                className="px-3 py-2 text-[10px] font-semibold uppercase tracking-wider"
                style={{ color: '#8888a0', borderBottom: '1px solid #2a2a3a' }}
              >
                Recent Vaults
              </div>
              {recentVaults.length === 0 && (
                <p className="px-3 py-3 text-xs" style={{ color: '#8888a0' }}>
                  No recent vaults yet.
                </p>
              )}
              {recentVaults.map((v) => {
                const isCurrent = vaultPath === v.path;
                return (
                  <button
                    key={v.path}
                    onClick={() => {
                      setVaultMenuOpen(false);
                      if (!isCurrent) onSwitchVault(v);
                    }}
                    className="w-full flex items-center gap-2 px-3 py-2 hover:bg-[#1a1a25] transition-colors text-left"
                    style={isCurrent ? { backgroundColor: '#181828' } : undefined}
                  >
                    <KindBadge kind={v.kind} size="sm" />
                    <div className="min-w-0 flex-1">
                      <p
                        className="truncate text-xs"
                        style={{ color: isCurrent ? kindMeta.color : '#e4e4ef', fontWeight: isCurrent ? 600 : 400 }}
                      >
                        {v.name}
                        {isCurrent && <span style={{ color: kindMeta.color }}> ● current</span>}
                      </p>
                      <p
                        className="truncate"
                        style={{
                          fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
                          fontSize: '9.5px',
                          color: '#8888a0',
                        }}
                        title={v.path}
                      >
                        {displayPath(v.path, homeDirPath)}
                      </p>
                    </div>
                    <span className="flex-shrink-0 text-[9px]" style={{ color: '#71717a' }}>
                      {formatRelativeTime(v.lastOpenedAt)}
                    </span>
                  </button>
                );
              })}

              <div style={{ borderTop: '1px solid #2a2a3a' }}>
                <button
                  onClick={() => {
                    setVaultMenuOpen(false);
                    onOpenOther();
                  }}
                  className="w-full px-3 py-2 text-left text-xs hover:bg-[#1a1a25] transition-colors"
                  style={{ color: '#a5b4fc' }}
                >
                  📂 Open Other Vault…
                </button>
                <button
                  onClick={() => {
                    setVaultMenuOpen(false);
                    onSetKind(vaultKind === 'business' ? 'private' : 'business');
                  }}
                  className="w-full px-3 py-2 text-left text-xs hover:bg-[#1a1a25] transition-colors"
                  style={{ color: '#a5b4fc' }}
                >
                  {vaultKind === 'business'
                    ? '🔒 Mark as Private'
                    : '💼 Mark as Business'}
                </button>
              </div>
            </div>
          )}
        </div>

        <span className="text-zinc-800">│</span>

        {/* Profile indicator */}
        <div className="flex items-center gap-1.5 flex-shrink-0">
          <div
            className="w-1.5 h-1.5 rounded-full flex-shrink-0"
            style={{ backgroundColor: kindMeta.color }}
          />
          <span className="truncate max-w-[120px]" title={profileName}>
            {profileName}
          </span>
        </div>

        <span className="text-zinc-800">│</span>

        {/* Note count */}
        <span className="flex-shrink-0">{formatNoteCount(noteCount)}</span>

        {/* Active note title */}
        {activeNoteTitle && (
          <>
            <span className="text-zinc-800">│</span>
            <span
              className="truncate max-w-[200px] text-zinc-500"
              title={activeNoteTitle}
            >
              {activeNoteTitle}
            </span>
          </>
        )}
      </div>

      {/* Right side */}
      <div className="flex items-center gap-3 flex-shrink-0">
        {/* Word count */}
        {wordCount > 0 && (
          <span className="text-zinc-500">{formatWordCount(wordCount)}</span>
        )}

        {/* Last saved */}
        {lastSaved && relativeTime && (
          <span className="text-zinc-600">
            Saved {relativeTime}
          </span>
        )}

        {/* Sync status */}
        <div className="flex items-center gap-1.5">
          <svg
            className="w-3 h-3 flex-shrink-0"
            viewBox="0 0 12 12"
            fill="none"
          >
            {syncState === 'syncing' ? (
              <circle
                cx="6"
                cy="6"
                r="4"
                stroke={dotColor}
                strokeWidth="1.5"
                strokeDasharray="6 3"
                className="animate-spin"
                style={{ transformOrigin: '6px 6px', animationDuration: '1.5s' }}
              />
            ) : (
              <circle
                cx="6"
                cy="6"
                r="3.5"
                fill={dotColor}
                opacity={0.9}
              />
            )}
          </svg>
          <span
            style={{
              color: syncState === 'error'
                ? '#f87171'
                : syncState === 'syncing'
                ? '#fbbf24'
                : syncState === 'synced'
                ? '#4ade80'
                : '#52525b',
            }}
          >
            {syncLabel}
          </span>
        </div>
      </div>
    </div>
  );
};

export default StatusBar;
