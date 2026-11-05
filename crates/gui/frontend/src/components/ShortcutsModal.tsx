import React from "react";

// ── Types ──────────────────────────────────────────────────────

interface ShortcutsModalProps {
  onClose: () => void;
}

interface ShortcutRow {
  keys: string[];
  label: string;
}

const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
const MOD = isMac ? "⌘" : "Ctrl";

interface ShortcutSection {
  title: string;
  rows: ShortcutRow[];
}

const SECTIONS: ShortcutSection[] = [
  {
    title: "General",
    rows: [
      { keys: [MOD, "K"], label: "Search notes" },
      { keys: [MOD, "N"], label: "New note" },
      { keys: [MOD, "S"], label: "Save note" },
      { keys: [MOD, "G"], label: "Knowledge graph" },
      { keys: [MOD, "⇧", "O"], label: "Import Obsidian vault" },
      { keys: ["?"], label: "Show this help" },
      { keys: ["Esc"], label: "Close dialogs" },
    ],
  },
  {
    title: "Editor & views",
    rows: [
      { keys: [MOD, "E"], label: "Cycle view: edit → split → preview" },
      { keys: [MOD, "P"], label: "Toggle preview" },
      { keys: [MOD, "F"], label: "Find in note" },
      { keys: [MOD, "H"], label: "Find & replace" },
      { keys: ["F3", "⇧F3"], label: "Next / previous match" },
      { keys: ["Enter"], label: "Jump to next match" },
      { keys: ["Esc"], label: "Clear search highlight" },
    ],
  },
  {
    title: "Font size",
    rows: [
      { keys: [MOD, "+"], label: "Increase editor font" },
      { keys: [MOD, "−"], label: "Decrease editor font" },
      { keys: [MOD, "0"], label: "Reset editor font" },
    ],
  },
  {
    title: "Import",
    rows: [
      { keys: ["Drag", "&", "Drop"], label: "Drop files onto the window to import" },
    ],
  },
];

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd
      className="inline-flex items-center justify-center rounded border border-[#2a2a3a] bg-[#1e1e2e] text-[#c8c8e0]"
      style={{
        minWidth: 22,
        height: 20,
        padding: "0 6px",
        fontSize: 11,
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
        boxShadow: "0 1px 0 rgba(0,0,0,0.4)",
      }}
    >
      {children}
    </kbd>
  );
}

// ── Component ──────────────────────────────────────────────────

export default function ShortcutsModal({ onClose }: ShortcutsModalProps) {
  return (
    <div
      className="fixed inset-0 bg-black/70 flex items-center justify-center z-[60]"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="bg-[#12121a] rounded-xl border border-[#2a2a3a] w-[560px] max-h-[76vh] flex flex-col shadow-2xl animate-fade-in">
        <div className="flex items-center justify-between px-5 py-4 border-b border-[#2a2a3a]">
          <div className="flex items-center gap-2.5">
            <span className="text-lg">⌨️</span>
            <h2 className="text-lg font-semibold text-white">Keyboard Shortcuts</h2>
          </div>
          <button onClick={onClose} className="text-gray-400 hover:text-white text-2xl leading-none">
            &times;
          </button>
        </div>

        <div className="overflow-auto px-5 py-4 grid grid-cols-2 gap-x-6 gap-y-5">
          {SECTIONS.map((section) => (
            <div key={section.title} className={section.title === "Import" ? "col-span-2" : ""}>
              <p className="text-[10px] font-semibold uppercase tracking-wider text-[#8888a0] mb-2">
                {section.title}
              </p>
              <div className="space-y-1.5">
                {section.rows.map((row) => (
                  <div key={row.label} className="flex items-center justify-between gap-3">
                    <span className="text-xs text-[#c8c8e0]">{row.label}</span>
                    <span className="flex items-center gap-1 flex-shrink-0">
                      {row.keys.map((k, i) =>
                        k === "&" ? (
                          <span key={i} className="text-[10px] text-[#55556a]">+</span>
                        ) : (
                          <Kbd key={i}>{k}</Kbd>
                        ),
                      )}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>

        <div className="px-5 py-3 border-t border-[#2a2a3a] text-[11px] text-[#71717a]">
          Tip: press <Kbd>?</Kbd> anywhere (outside the editor) to reopen this panel.
        </div>
      </div>
    </div>
  );
}
