import type { TrackId } from "./types";

/**
 * Row selection by track id, so sorting and filtering keep it. `cursor` is
 * the focused row; `anchor` is where shift-ranges start.
 */
export interface SelectionState {
  selected: ReadonlySet<TrackId>;
  anchor: TrackId | null;
  cursor: TrackId | null;
}

export const EMPTY_SELECTION: SelectionState = {
  selected: new Set(),
  anchor: null,
  cursor: null,
};

export type SelectionAction =
  /** Plain click: select only this row. */
  | { type: "select"; id: TrackId }
  /** Cmd-click: add or remove one row. */
  | { type: "toggle"; id: TrackId }
  /** Shift-click: select from the anchor to this row. */
  | { type: "range"; id: TrackId; order: readonly TrackId[] }
  /** Arrow keys: move the cursor, extending the range with shift. */
  | {
      type: "move";
      delta: number;
      order: readonly TrackId[];
      extend: boolean;
    }
  | { type: "all"; order: readonly TrackId[] }
  | { type: "clear" };

function rangeBetween(
  order: readonly TrackId[],
  from: TrackId,
  to: TrackId,
): Set<TrackId> {
  const a = order.indexOf(from);
  const b = order.indexOf(to);
  if (a < 0 || b < 0) return new Set([to]);
  const [start, end] = a <= b ? [a, b] : [b, a];
  return new Set(order.slice(start, end + 1));
}

export function selectionReducer(
  state: SelectionState,
  action: SelectionAction,
): SelectionState {
  switch (action.type) {
    case "select":
      return {
        selected: new Set([action.id]),
        anchor: action.id,
        cursor: action.id,
      };
    case "toggle": {
      const selected = new Set(state.selected);
      if (selected.has(action.id)) selected.delete(action.id);
      else selected.add(action.id);
      return { selected, anchor: action.id, cursor: action.id };
    }
    case "range": {
      const anchor = state.anchor ?? state.cursor ?? action.id;
      return {
        selected: rangeBetween(action.order, anchor, action.id),
        anchor,
        cursor: action.id,
      };
    }
    case "move": {
      const { order } = action;
      if (order.length === 0) return state;
      const current = state.cursor === null ? -1 : order.indexOf(state.cursor);
      let index: number;
      if (current < 0) index = action.delta >= 0 ? 0 : order.length - 1;
      else
        index = Math.min(order.length - 1, Math.max(0, current + action.delta));
      const cursor = order[index];
      if (action.extend) {
        const anchor =
          state.anchor !== null && order.includes(state.anchor)
            ? state.anchor
            : (state.cursor ?? cursor);
        return {
          selected: rangeBetween(order, anchor, cursor),
          anchor,
          cursor,
        };
      }
      return { selected: new Set([cursor]), anchor: cursor, cursor };
    }
    case "all":
      return {
        selected: new Set(action.order),
        anchor: state.anchor,
        cursor: state.cursor ?? action.order[0] ?? null,
      };
    case "clear":
      return EMPTY_SELECTION;
  }
}

/** Selected ids that are visible, in table order. */
export function selectedInOrder(
  selection: SelectionState,
  order: readonly TrackId[],
): TrackId[] {
  if (selection.selected.size === 0) return [];
  return order.filter((id) => selection.selected.has(id));
}
