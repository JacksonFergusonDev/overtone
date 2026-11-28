import { Search, X } from "lucide-react";
import { useHotkeys } from "react-hotkeys-hook";
import { useRef } from "react";
import type { MissingFilter } from "../lib/filter";
import { VERDICT_LABEL } from "../lib/format";
import type { QualityVerdict } from "../lib/types";
import { useLibrary } from "../store/library";
import { useVibeMap, useVisibleIds } from "../store/selectors";
import { showVibe, updateTable, useUi, type TableKey } from "../store/ui";
import { TrackTable } from "./TrackTable";

const VERDICTS: QualityVerdict[] = [
  "suspect",
  "warning",
  "inconclusive",
  "unchecked",
  "pass",
];

function FilterBar({ tableKey }: { tableKey: TableKey }) {
  const table = useUi((s) => s.tables[tableKey]);
  const damagedCount = useLibrary((s) => s.damaged.size);
  const view = useUi((s) => s.view);
  const vibeFilter = useUi((s) => s.vibeFilter);
  const vibes = useVibeMap();
  const inputRef = useRef<HTMLInputElement | null>(null);

  useHotkeys(
    ["slash", "mod+f"],
    () => {
      inputRef.current?.focus();
      inputRef.current?.select();
    },
    { scopes: "global", enabled: view === tableKey, preventDefault: true },
  );

  const toggleVerdict = (verdict: QualityVerdict) => {
    const verdicts = table.verdicts.includes(verdict)
      ? table.verdicts.filter((v) => v !== verdict)
      : [...table.verdicts, verdict];
    updateTable(tableKey, { verdicts });
  };

  const vibe =
    tableKey === "library" && vibeFilter !== null
      ? vibes.get(vibeFilter)
      : undefined;

  return (
    <div className="filter-bar" role="search">
      <label className="search">
        <Search size={14} aria-hidden="true" />
        <input
          ref={inputRef}
          type="search"
          aria-label="Search title, artist, album, or comment"
          placeholder="Search"
          value={table.query}
          onChange={(event) =>
            updateTable(tableKey, { query: event.target.value })
          }
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              if (table.query) updateTable(tableKey, { query: "" });
              else event.currentTarget.blur();
            } else if (event.key === "Enter" || event.key === "ArrowDown") {
              // Jump into the results.
              event.preventDefault();
              event.currentTarget
                .closest(".view")
                ?.querySelector<HTMLElement>("[role=grid]")
                ?.focus();
            }
          }}
        />
      </label>
      {tableKey === "library" && (
        <div className="chips" role="group" aria-label="Filter by verdict">
          {VERDICTS.map((verdict) => (
            <button
              key={verdict}
              type="button"
              className={`chip verdict-chip verdict-${verdict}`}
              aria-pressed={table.verdicts.includes(verdict)}
              onClick={() => toggleVerdict(verdict)}
            >
              {VERDICT_LABEL[verdict]}
            </button>
          ))}
        </div>
      )}
      <button
        type="button"
        className="chip"
        aria-pressed={table.missing === "missing"}
        onClick={() =>
          updateTable(tableKey, {
            missing: (table.missing === "missing"
              ? "any"
              : "missing") as MissingFilter,
          })
        }
      >
        Missing
      </button>
      {(damagedCount > 0 || table.damaged) && (
        <button
          type="button"
          className="chip"
          aria-pressed={table.damaged}
          title="Tracks whose integrity check found decode errors in the music"
          onClick={() => updateTable(tableKey, { damaged: !table.damaged })}
        >
          Damaged <span className="chip-count">{damagedCount}</span>
        </button>
      )}
      {vibe && (
        <span className="chip chip-vibe">
          <span className="truncate" title={vibe.path}>
            Vibe: {vibe.path}
          </span>
          <button
            type="button"
            className="chip-clear"
            aria-label={`Stop filtering by ${vibe.path}`}
            onClick={() => showVibe(null)}
          >
            <X size={12} aria-hidden="true" />
          </button>
        </span>
      )}
    </div>
  );
}

export function LibraryView({ tableKey }: { tableKey: TableKey }) {
  const ids = useVisibleIds(tableKey);
  const total = useLibrary((s) => s.order.length);
  const selectedCount = useUi(
    (s) => s.tables[tableKey].selection.selected.size,
  );
  const vibeFilter = useUi((s) => s.vibeFilter);
  const vibes = useVibeMap();
  const vibe =
    tableKey === "library" && vibeFilter !== null
      ? vibes.get(vibeFilter)
      : undefined;
  const title =
    tableKey === "review" ? "Needs review" : (vibe?.path ?? "Library");

  let empty: string | null = null;
  if (ids.length === 0) {
    if (total === 0)
      empty = "Your library is empty. Add a folder in Settings, then scan it.";
    else if (tableKey === "review")
      empty =
        "Nothing needs review. Flagged tracks appear here after an audit.";
    else empty = "No tracks match these filters.";
  }

  return (
    <div className="view table-view">
      <header className="view-header">
        <h1 className="view-title truncate" title={title}>
          {title}
        </h1>
        <span className="view-count num" aria-live="polite">
          {ids.length.toLocaleString()} {ids.length === 1 ? "track" : "tracks"}
          {selectedCount > 0 && ` · ${selectedCount.toLocaleString()} selected`}
        </span>
      </header>
      <FilterBar tableKey={tableKey} />
      {empty ? (
        <p className="empty">{empty}</p>
      ) : (
        <TrackTable tableKey={tableKey} ids={ids} label={title} />
      )}
    </div>
  );
}
