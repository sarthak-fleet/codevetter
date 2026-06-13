import { useCallback, useEffect, useMemo,useRef, useState } from "react";
import { useNavigate } from "react-router-dom";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { getPreference,setPreference } from "@/lib/tauri-ipc";

// ─── Types ──────────────────────────────────────────────────────────────────

interface CommandItem {
  id: string;
  label: string;
  description?: string;
  icon: string;
  shortcut?: string;
  group: string;
  action: () => void;
}

interface CommandPaletteProps {
  isOpen: boolean;
  onClose: () => void;
}

// ─── Fuzzy matching ─────────────────────────────────────────────────────────

function matchScore(query: string, text: string): number {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (!t.includes(q)) return -1;
  if (t.startsWith(q)) return 2;
  if (t.indexOf(q) > 0) return 1;
  return 0;
}

function filterAndSort(items: CommandItem[], query: string): CommandItem[] {
  if (!query.trim()) return items;

  const q = query.toLowerCase();
  const scored: { item: CommandItem; score: number }[] = [];

  for (const item of items) {
    const labelScore = matchScore(q, item.label);
    const descScore = item.description ? matchScore(q, item.description) : -1;
    const best = Math.max(labelScore, descScore);
    if (best >= 0) {
      // Prefer label matches over description matches
      scored.push({ item, score: labelScore >= 0 ? best + 1 : best });
    }
  }

  scored.sort((a, b) => b.score - a.score);
  return scored.map((s) => s.item);
}

// ─── Component ──────────────────────────────────────────────────────────────

export default function CommandPalette({ isOpen, onClose }: CommandPaletteProps) {
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // Build the command list (stable across renders via navigate reference)
  const commands: CommandItem[] = useMemo(() => {
    function go(path: string) {
      return () => {
        navigate(path);
        onClose();
      };
    }

    return [
      // Navigation
      { id: "nav-home", label: "Go to Home", icon: "\u2302", shortcut: "g h", group: "Navigation", action: go("/") },
      { id: "nav-review", label: "Go to Review", icon: "\u2714", shortcut: "g r", group: "Navigation", action: go("/review") },
      { id: "nav-roadmap", label: "Go to Roadmap", icon: "\u25ce", shortcut: "g m", group: "Navigation", action: go("/roadmap") },
      { id: "nav-settings", label: "Go to Settings", icon: "\u2638", shortcut: "g ,", group: "Navigation", action: go("/settings") },

      // Actions
      { id: "act-start-review", label: "Start Review", description: "Run a code review", icon: "\u2714", group: "Actions", action: go("/review") },
    ];
  }, [navigate, onClose]);

  const filtered = useMemo(() => filterAndSort(commands, query), [commands, query]);

  // Reset state when palette opens
  useEffect(() => {
    if (isOpen) {
      setQuery("");
      setSelectedIndex(0);
      // Small delay so the DOM is painted before we focus
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [isOpen]);

  // Clamp selectedIndex when filtered list shrinks
  useEffect(() => {
    if (selectedIndex >= filtered.length) {
      setSelectedIndex(Math.max(0, filtered.length - 1));
    }
  }, [filtered.length, selectedIndex]);

  // Scroll selected item into view
  useEffect(() => {
    if (!listRef.current) return;
    const el = listRef.current.querySelector(`[data-index="${selectedIndex}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          setSelectedIndex((i) => (i + 1) % Math.max(1, filtered.length));
          break;
        case "ArrowUp":
          e.preventDefault();
          setSelectedIndex((i) => (i - 1 + filtered.length) % Math.max(1, filtered.length));
          break;
        case "Enter":
          e.preventDefault();
          if (filtered[selectedIndex]) {
            filtered[selectedIndex].action();
          }
          break;
        case "Escape":
          e.preventDefault();
          onClose();
          break;
      }
    },
    [filtered, selectedIndex, onClose]
  );

  // Group the filtered items for rendering
  const groups: { name: string; items: { item: CommandItem; globalIndex: number }[] }[] = [];
  const groupMap = new Map<string, { item: CommandItem; globalIndex: number }[]>();

  filtered.forEach((item, idx) => {
    let arr = groupMap.get(item.group);
    if (!arr) {
      arr = [];
      groupMap.set(item.group, arr);
      groups.push({ name: item.group, items: arr });
    }
    arr.push({ item, globalIndex: idx });
  });

  return (
    <Dialog open={isOpen} onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent
        hideClose
        className="max-w-lg mx-4 p-0 bg-[#0a0a0a] border border-[#1a1a1a] rounded-xl shadow-2xl overflow-hidden top-[30%] translate-y-0"
        onKeyDown={handleKeyDown}
      >
        {/* Search input */}
        <div className="flex items-center gap-3 px-4 py-3 border-b border-[#1a1a1a]">
          <span className="text-slate-500 text-sm">{"\u2315"}</span>
          <Input
            ref={inputRef}
            type="text"
            className="flex-1 bg-transparent text-lg text-slate-100 placeholder-slate-600 outline-none border-none shadow-none focus-visible:ring-0 h-auto p-0"
            placeholder="Search commands..."
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setSelectedIndex(0);
            }}
          />
          <Badge variant="outline" className="text-[10px] font-mono text-slate-600 bg-[#1a1a1a] rounded px-1.5 py-0.5">
            ESC
          </Badge>
        </div>

        {/* Results */}
        <div ref={listRef} className="max-h-[360px] overflow-y-auto py-2">
          {filtered.length === 0 && (
            <div className="px-4 py-8 text-center text-sm text-slate-600">
              No matching commands
            </div>
          )}

          {groups.map((group) => (
            <div key={group.name}>
              <div className="text-[10px] uppercase tracking-wider text-slate-600 px-3 py-1 select-none">
                {group.name}
              </div>
              {group.items.map(({ item, globalIndex }) => {
                const isSelected = globalIndex === selectedIndex;
                return (
                  <Button
                    key={item.id}
                    variant="ghost"
                    data-index={globalIndex}
                    className={`w-full flex items-center gap-3 px-3 py-2 h-auto text-left justify-start rounded-none transition-colors cursor-pointer ${
                      isSelected
                        ? "bg-amber-500/10 text-slate-100"
                        : "text-slate-400 hover:bg-[#111111]"
                    }`}
                    onClick={() => item.action()}
                    onMouseEnter={() => setSelectedIndex(globalIndex)}
                  >
                    <span className="w-5 text-center text-sm shrink-0">{item.icon}</span>
                    <span className="flex-1 text-[13px] font-medium truncate">{item.label}</span>
                    {item.description && (
                      <span className="text-[11px] text-slate-600 truncate max-w-[160px]">
                        {item.description}
                      </span>
                    )}
                    {item.shortcut && (
                      <Badge variant="outline" className="text-[11px] text-slate-600 font-mono shrink-0 rounded px-1 py-0">
                        {item.shortcut}
                      </Badge>
                    )}
                  </Button>
                );
              })}
            </div>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}
