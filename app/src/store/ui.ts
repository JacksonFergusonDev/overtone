import { create } from "zustand";
import type { MissingFilter } from "../lib/filter";
import {
  EMPTY_SELECTION,
  selectionReducer,
  type SelectionAction,
  type SelectionState,
} from "../lib/selection";
import type { SortSpec } from "../lib/sort";
import type { QualityVerdict, TrackId, VibeId } from "../lib/types";

export type View = "library" | "review" | "sync" | "settings";
export type TableKey = "library" | "review";
export type Modal = "palette" | "shortcuts" | "confirm" | null;

export interface TableState {
  sort: SortSpec;
  query: string;
  verdicts: readonly QualityVerdict[];
  missing: MissingFilter;
  /** Only tracks whose integrity check found errors in the music. */
  damaged: boolean;
  selection: SelectionState;
}

export interface UiState {
  view: View;
  /** Views shown at least once; they stay mounted and are hidden when inactive. */
  visited: ReadonlySet<View>;
  /** Vibe chosen in the sidebar; filters the library to it and its children. */
  vibeFilter: VibeId | null;
  tables: Record<TableKey, TableState>;
  inspectorOpen: boolean;
  modal: Modal;
  /** Most recently toggled vibes, newest first. */
  recentVibes: readonly VibeId[];
}

const table = (sort: SortSpec): TableState => ({
  sort,
  query: "",
  verdicts: [],
  missing: "any",
  damaged: false,
  selection: EMPTY_SELECTION,
});

const initialState: UiState = {
  view: "library",
  visited: new Set<View>(["library"]),
  vibeFilter: null,
  tables: {
    library: table({ key: "artist", dir: "asc" }),
    review: table({ key: "quality", dir: "asc" }),
  },
  inspectorOpen: false,
  modal: null,
  recentVibes: [],
};

export const useUi = create<UiState>(() => initialState);

export function resetUi(): void {
  useUi.setState(initialState, true);
}

export function setView(view: View): void {
  useUi.setState((state) => ({
    view,
    visited: state.visited.has(view)
      ? state.visited
      : new Set([...state.visited, view]),
    // The review queue is driven from the inspector.
    inspectorOpen: view === "review" ? true : state.inspectorOpen,
  }));
}

/** Shows the library filtered to a vibe (or every track for null). */
export function showVibe(vibeId: VibeId | null): void {
  setView("library");
  useUi.setState({ vibeFilter: vibeId });
}

/** The table the keyboard acts on, if a table view is showing. */
export function activeTable(
  state: UiState = useUi.getState(),
): TableKey | null {
  return state.view === "library" || state.view === "review"
    ? state.view
    : null;
}

export function updateTable(key: TableKey, patch: Partial<TableState>): void {
  useUi.setState((state) => ({
    tables: { ...state.tables, [key]: { ...state.tables[key], ...patch } },
  }));
}

export function dispatchSelection(
  key: TableKey,
  action: SelectionAction,
): void {
  useUi.setState((state) => {
    const current = state.tables[key];
    const selection = selectionReducer(current.selection, action);
    if (selection === current.selection) return state;
    return { tables: { ...state.tables, [key]: { ...current, selection } } };
  });
}

export function toggleInspector(): void {
  useUi.setState((state) => ({ inspectorOpen: !state.inspectorOpen }));
}

export function setInspectorOpen(open: boolean): void {
  useUi.setState({ inspectorOpen: open });
}

export function openModal(modal: Exclude<Modal, null>): void {
  useUi.setState({ modal });
}

export function closeModal(): void {
  useUi.setState({ modal: null });
}

export function rememberVibe(vibeId: VibeId): void {
  useUi.setState((state) => ({
    recentVibes: [
      vibeId,
      ...state.recentVibes.filter((id) => id !== vibeId),
    ].slice(0, 9),
  }));
}

/** The focused row in a table, which the inspector shows. */
export function focusedTrackId(state: UiState, key: TableKey): TrackId | null {
  return state.tables[key].selection.cursor;
}
