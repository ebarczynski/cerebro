import React from 'react';

interface StatusBarProps {
  noteCount: number;
  wordCount: number;
  lastSaved: Date | null;
  profileName: string;
  activeNoteTitle: string | null;
  syncStatus: string;
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

function formatRelativeTime(date: Date | null): string {
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

const StatusBar: React.FC<StatusBarProps> = ({
  noteCount,
  wordCount,
  lastSaved,
  profileName,
  activeNoteTitle,
  syncStatus,
}) => {
  const syncState = getSyncState(syncStatus);
  const dotColor = getSyncDotColor(syncState);
  const syncLabel = getSyncLabel(syncState);
  const relativeTime = formatRelativeTime(lastSaved);

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
        {/* Profile indicator */}
        <div className="flex items-center gap-1.5 flex-shrink-0">
          <div
            className="w-1.5 h-1.5 rounded-full flex-shrink-0"
            style={{ backgroundColor: '#6366f1' }}
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
