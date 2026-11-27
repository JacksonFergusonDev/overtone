import { useVirtualizer, type Rect } from "@tanstack/react-virtual";
import { AlertCircle, Volume2 } from "lucide-react";
import {
  createContext,
  memo,
  useCallback,
  useContext,
  useEffect,
  useRef,
  type DragEvent,
  type MouseEvent,
} from "react";
import { useHotkeys } from "react-hotkeys-hook";
import {
  formatAudio,
  formatBpm,
  formatDuration,
  trackTitle,
} from "../lib/format";
import { nextSort, type SortKey } from "../lib/sort";
import type { Track, TrackId, Vibe, VibeId } from "../lib/types";
import { useLibrary } from "../store/library";
import { playTrack, usePlayer } from "../store/player";
import { currentVisibleIds, useVibeMap } from "../store/selectors";
import {
  dispatchSelection,
  updateTable,
  useUi,
  type TableKey,
} from "../store/ui";
import { VerdictBadge, VibePills } from "./Badges";

export const ROW_HEIGHT = 30;

/**
 * Lets tests give the virtualizer a fixed viewport, since jsdom has no
 * layout and would otherwise render no rows.
 */
export const TableRectContext = createContext<Rect | null>(null);

export const TRACK_DRAG_TYPE = "application/x-overtone-tracks";

interface Column {
  id: string;
  label: string;
  sort: SortKey | null;
  className: string;
}

const COLUMNS: Column[] = [
  { id: "state", label: "", sort: null, className: "col-state" },
  { id: "title", label: "Title", sort: "title", className: "col-title" },
  { id: "artist", label: "Artist", sort: "artist", className: "col-artist" },
  { id: "bpm", label: "BPM", sort: "bpm", className: "col-bpm num" },
  { id: "key", label: "Key", sort: "key", className: "col-key" },
  { id: "format", label: "Format", sort: "format", className: "col-format" },
  {
    id: "duration",
    label: "Time",
    sort: "duration",
    className: "col-time num",
  },
  {
    id: "quality",
    label: "Quality",
    sort: "quality",
    className: "col-quality",
  },
  { id: "vibes", label: "Vibes", sort: null, className: "col-vibes" },
];

export const rowDomId = (key: TableKey, id: TrackId): string =>
  `${key}-row-${id}`;

interface RowProps {
  track: Track;
  tableKey: TableKey;
  index: number;
  start: number;
  selected: boolean;
  focused: boolean;
  playing: boolean;
  vibes: ReadonlyMap<VibeId, Vibe>;
}

const TrackRow = memo(function TrackRow({
  track,
  tableKey,
  index,
  start,
  selected,
  focused,
  playing,
  vibes,
}: RowProps) {
  const title = trackTitle(track);
  const artist = track.tags.artist ?? "";
  const audio = formatAudio(track.properties);
  const missing = track.missing_since !== null;
  const classes = ["row", "track-grid"];
  if (selected) classes.push("selected");
  if (focused) classes.push("focused");
  if (missing) classes.push("missing");
  if (playing) classes.push("playing");
  return (
    <div
      role="row"
      id={rowDomId(tableKey, track.id)}
      className={classes.join(" ")}
      aria-selected={selected}
      aria-rowindex={index + 2}
      data-id={track.id}
      draggable
      style={{ transform: `translateY(${start}px)` }}
    >
      <span role="gridcell" className="cell col-state">
        {missing ? (
          <AlertCircle
            size={14}
            aria-label="File missing"
            className="icon-missing"
          />
        ) : playing ? (
          <Volume2
            size={14}
            aria-label="Now playing"
            className="icon-playing"
          />
        ) : null}
      </span>
      <span role="gridcell" className="cell col-title" title={title}>
        {title}
      </span>
      <span role="gridcell" className="cell col-artist" title={artist}>
        {artist}
      </span>
      <span role="gridcell" className="cell col-bpm num">
        {formatBpm(track.tags.bpm)}
      </span>
      <span role="gridcell" className="cell col-key">
        {track.tags.key ?? ""}
      </span>
      <span
        role="gridcell"
        className="cell col-format"
        title={`${audio.label} ${audio.detail}`.trim()}
      >
        {audio.label}
        {audio.detail && <span className="dim"> {audio.detail}</span>}
      </span>
      <span role="gridcell" className="cell col-time num">
        {formatDuration(track.properties.duration_ms)}
      </span>
      <span role="gridcell" className="cell col-quality">
        <VerdictBadge quality={track.quality} />
      </span>
      <span role="gridcell" className="cell col-vibes">
        <VibePills ids={track.vibes} vibes={vibes} />
      </span>
    </div>
  );
});

export interface TrackTableProps {
  tableKey: TableKey;
  ids: readonly TrackId[];
  label: string;
}

export function TrackTable({ tableKey, ids, label }: TrackTableProps) {
  const tracks = useLibrary((s) => s.tracks);
  const vibes = useVibeMap();
  const sort = useUi((s) => s.tables[tableKey].sort);
  const selection = useUi((s) => s.tables[tableKey].selection);
  const currentId = usePlayer((s) => s.currentId);
  const fixedRect = useContext(TableRectContext);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  const virtualizer = useVirtualizer({
    count: ids.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
    getItemKey: (index) => ids[index],
    ...(fixedRect
      ? {
          initialRect: fixedRect,
          observeElementRect: (_: unknown, cb: (rect: Rect) => void) => {
            cb(fixedRect);
            return undefined;
          },
        }
      : {}),
  });

  const cursor = selection.cursor;
  const cursorIndex = cursor === null ? -1 : ids.indexOf(cursor);
  useEffect(() => {
    if (cursorIndex >= 0)
      virtualizer.scrollToIndex(cursorIndex, { align: "auto" });
  }, [cursorIndex, virtualizer]);

  const pageSize = () =>
    Math.max(
      1,
      Math.floor(
        (scrollRef.current?.clientHeight || ROW_HEIGHT * 10) / ROW_HEIGHT,
      ) - 1,
    );

  const gridRef = useHotkeys<HTMLDivElement>(
    [
      "up",
      "down",
      "shift+up",
      "shift+down",
      "home",
      "end",
      "shift+home",
      "shift+end",
      "pageup",
      "pagedown",
      "shift+pageup",
      "shift+pagedown",
      "enter",
      "mod+a",
      "escape",
    ],
    (event) => {
      const order = currentVisibleIds(tableKey);
      const extend = event.shiftKey;
      const move = (delta: number) =>
        dispatchSelection(tableKey, { type: "move", delta, order, extend });
      switch (event.code) {
        case "ArrowUp":
          return move(-1);
        case "ArrowDown":
          return move(1);
        case "Home":
          return move(-order.length);
        case "End":
          return move(order.length);
        case "PageUp":
          return move(-pageSize());
        case "PageDown":
          return move(pageSize());
        case "Enter": {
          const id = useUi.getState().tables[tableKey].selection.cursor;
          if (id !== null) playTrack(id);
          return;
        }
        case "KeyA":
          return dispatchSelection(tableKey, { type: "all", order });
        case "Escape":
          return dispatchSelection(tableKey, { type: "clear" });
      }
    },
    { scopes: "table", preventDefault: true },
  );

  const rowIdFromEvent = (event: MouseEvent | DragEvent): TrackId | null => {
    const row = (event.target as HTMLElement).closest<HTMLElement>("[data-id]");
    return row ? Number(row.dataset.id) : null;
  };

  const onClick = useCallback(
    (event: MouseEvent) => {
      const id = rowIdFromEvent(event);
      if (id === null) return;
      if (event.shiftKey) {
        dispatchSelection(tableKey, {
          type: "range",
          id,
          order: currentVisibleIds(tableKey),
        });
      } else if (event.metaKey || event.ctrlKey) {
        dispatchSelection(tableKey, { type: "toggle", id });
      } else {
        dispatchSelection(tableKey, { type: "select", id });
      }
    },
    [tableKey],
  );

  const onDoubleClick = useCallback((event: MouseEvent) => {
    const id = rowIdFromEvent(event);
    if (id !== null) playTrack(id);
  }, []);

  const onDragStart = useCallback(
    (event: DragEvent) => {
      const id = rowIdFromEvent(event);
      if (id === null) return;
      let dragged: TrackId[];
      const current = useUi.getState().tables[tableKey].selection;
      if (current.selected.has(id)) {
        dragged = currentVisibleIds(tableKey).filter((t) =>
          current.selected.has(t),
        );
      } else {
        dispatchSelection(tableKey, { type: "select", id });
        dragged = [id];
      }
      event.dataTransfer.setData(TRACK_DRAG_TYPE, JSON.stringify(dragged));
      event.dataTransfer.setData(
        "text/plain",
        `${dragged.length} ${dragged.length === 1 ? "track" : "tracks"}`,
      );
      event.dataTransfer.effectAllowed = "copy";
    },
    [tableKey],
  );

  const activeDescendant =
    cursor !== null && cursorIndex >= 0
      ? rowDomId(tableKey, cursor)
      : undefined;

  return (
    <div
      ref={gridRef}
      className="table"
      role="grid"
      aria-label={label}
      aria-rowcount={ids.length + 1}
      aria-colcount={COLUMNS.length}
      aria-multiselectable="true"
      aria-activedescendant={activeDescendant}
      tabIndex={0}
    >
      <div role="rowgroup" className="table-head">
        <div role="row" className="track-grid head-row" aria-rowindex={1}>
          {COLUMNS.map((column) => {
            const sorted = column.sort !== null && sort.key === column.sort;
            return (
              <span
                key={column.id}
                role="columnheader"
                className={`cell ${column.className}`}
                aria-sort={
                  sorted
                    ? sort.dir === "asc"
                      ? "ascending"
                      : "descending"
                    : undefined
                }
              >
                {column.sort ? (
                  <button
                    type="button"
                    className="sort-button"
                    tabIndex={-1}
                    onClick={() =>
                      updateTable(tableKey, {
                        sort: nextSort(sort, column.sort as SortKey),
                      })
                    }
                  >
                    {column.label}
                    {sorted && (
                      <span aria-hidden="true" className="sort-arrow">
                        {sort.dir === "asc" ? "▲" : "▼"}
                      </span>
                    )}
                  </button>
                ) : (
                  <span className="visually-hidden-empty">{column.label}</span>
                )}
              </span>
            );
          })}
        </div>
      </div>
      <div
        ref={scrollRef}
        role="rowgroup"
        className="table-body"
        onClick={onClick}
        onDoubleClick={onDoubleClick}
        onDragStart={onDragStart}
      >
        <div
          className="table-spacer"
          style={{ height: virtualizer.getTotalSize() }}
        >
          {virtualizer.getVirtualItems().map((item) => {
            const id = ids[item.index];
            const track = tracks.get(id);
            if (!track) return null;
            return (
              <TrackRow
                key={id}
                track={track}
                tableKey={tableKey}
                index={item.index}
                start={item.start}
                selected={selection.selected.has(id)}
                focused={cursor === id}
                playing={currentId === id}
                vibes={vibes}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}
