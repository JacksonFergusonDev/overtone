import {
  ArrowLeftRight,
  Flag,
  Library,
  Pencil,
  Plus,
  Settings,
  Trash2,
} from "lucide-react";
import { useMemo, useState, type DragEvent, type ReactNode } from "react";
import { errorMessage } from "../lib/backend";
import type { Vibe } from "../lib/types";
import {
  parseVibePath,
  vibeName,
  vibeTree,
  vibeWithDescendants,
} from "../lib/vibes";
import { addVibeToTracks, goTo } from "../store/actions";
import { confirmAction } from "../store/confirm";
import {
  createVibe,
  deleteVibe,
  renameVibe,
  useLibrary,
} from "../store/library";
import { useNeedsReviewCount, useVibeCounts } from "../store/selectors";
import { notifyError } from "../store/toasts";
import { showVibe, useUi, type View } from "../store/ui";
import { TRACK_DRAG_TYPE } from "./TrackTable";

function NavItem({
  view,
  icon,
  label,
  count,
  highlight,
}: {
  view: View;
  icon: ReactNode;
  label: string;
  count?: number;
  highlight?: boolean;
}) {
  const current = useUi(
    (s) => s.view === view && (view !== "library" || s.vibeFilter === null),
  );
  return (
    <button
      type="button"
      className="nav-item"
      aria-current={current ? "page" : undefined}
      onClick={() => goTo(view)}
    >
      {icon}
      <span className="nav-label">{label}</span>
      {count !== undefined && (
        <span
          className={`nav-count num${highlight && count > 0 ? " attention" : ""}`}
        >
          {count.toLocaleString()}
        </span>
      )}
    </button>
  );
}

/** Inline text field for creating or renaming a vibe. */
function VibeNameInput({
  initial,
  label,
  onSubmit,
  onCancel,
}: {
  initial: string;
  label: string;
  onSubmit: (path: string) => Promise<void>;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const submit = async () => {
    const parsed = parseVibePath(value);
    if (!parsed.ok) {
      setError(parsed.error);
      return;
    }
    try {
      await onSubmit(parsed.path);
    } catch (err) {
      setError(errorMessage(err));
    }
  };
  return (
    <div className="vibe-input">
      <input
        autoFocus
        aria-label={label}
        aria-invalid={error ? true : undefined}
        aria-describedby={error ? "vibe-input-error" : undefined}
        value={value}
        placeholder="Name, or Parent/Child"
        onChange={(event) => {
          setValue(event.target.value);
          setError(null);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") void submit();
          if (event.key === "Escape") onCancel();
        }}
        onBlur={() => {
          if (!value.trim() || value === initial) onCancel();
        }}
      />
      {error && (
        <p id="vibe-input-error" className="error-text small" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

function VibeRow({
  vibe,
  depth,
  count,
  onRename,
}: {
  vibe: Vibe;
  depth: number;
  count: number;
  onRename: () => void;
}) {
  const selected = useUi(
    (s) => s.view === "library" && s.vibeFilter === vibe.id,
  );
  const [dropping, setDropping] = useState(false);

  const remove = async () => {
    const nested =
      vibeWithDescendants(useLibrary.getState().vibes, vibe.id).size - 1;
    const ok = await confirmAction({
      title: `Delete “${vibe.path}”?`,
      body:
        (nested > 0
          ? `This also deletes ${nested} nested ${nested === 1 ? "vibe" : "vibes"}. `
          : "") +
        "Tracks stay in your library; only this grouping is removed. Crates synced from it are removed on the next sync.",
      confirmLabel: "Delete vibe",
      danger: true,
    });
    if (!ok) return;
    try {
      await deleteVibe(vibe.id);
      if (useUi.getState().vibeFilter === vibe.id) showVibe(null);
    } catch (error) {
      notifyError(`Couldn't delete the vibe: ${errorMessage(error)}`);
    }
  };

  const onDrop = (event: DragEvent) => {
    setDropping(false);
    const data = event.dataTransfer.getData(TRACK_DRAG_TYPE);
    if (!data) return;
    event.preventDefault();
    try {
      const ids = JSON.parse(data) as number[];
      void addVibeToTracks(ids, vibe.id);
    } catch {
      // Not our drag.
    }
  };

  return (
    <li
      className={`vibe-row${dropping ? " drop-target" : ""}`}
      style={{ paddingLeft: 8 + depth * 14 }}
      onDragOver={(event) => {
        if (event.dataTransfer.types.includes(TRACK_DRAG_TYPE)) {
          event.preventDefault();
          event.dataTransfer.dropEffect = "copy";
          setDropping(true);
        }
      }}
      onDragLeave={() => setDropping(false)}
      onDrop={onDrop}
    >
      <button
        type="button"
        className="vibe-button"
        aria-current={selected ? "page" : undefined}
        title={vibe.path}
        onClick={() => showVibe(vibe.id)}
      >
        <span className="truncate">{vibeName(vibe.path)}</span>
        <span className="nav-count num" aria-label={`${count} tracks`}>
          {count.toLocaleString()}
        </span>
      </button>
      <span className="vibe-actions">
        <button
          type="button"
          className="icon-button small"
          aria-label={`Rename ${vibe.path}`}
          onClick={onRename}
        >
          <Pencil size={12} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="icon-button small"
          aria-label={`Delete ${vibe.path}`}
          onClick={() => void remove()}
        >
          <Trash2 size={12} aria-hidden="true" />
        </button>
      </span>
    </li>
  );
}

export function Sidebar() {
  const vibes = useLibrary((s) => s.vibes);
  const total = useLibrary((s) => s.order.length);
  const reviewCount = useNeedsReviewCount();
  const counts = useVibeCounts();
  const tree = useMemo(() => vibeTree(vibes), [vibes]);
  const [creating, setCreating] = useState(false);
  const [renaming, setRenaming] = useState<number | null>(null);

  return (
    <nav className="sidebar" aria-label="Main">
      <div className="brand">Overtone</div>
      <NavItem
        view="library"
        icon={<Library size={16} aria-hidden="true" />}
        label="Library"
        count={total}
      />
      <NavItem
        view="review"
        icon={<Flag size={16} aria-hidden="true" />}
        label="Needs review"
        count={reviewCount}
        highlight
      />
      <div className="sidebar-section">
        <h2 className="sidebar-heading">Vibes</h2>
        <button
          type="button"
          className="icon-button small"
          aria-label="New vibe"
          onClick={() => setCreating(true)}
        >
          <Plus size={14} aria-hidden="true" />
        </button>
      </div>
      {creating && (
        <VibeNameInput
          initial=""
          label="New vibe name"
          onCancel={() => setCreating(false)}
          onSubmit={async (path) => {
            await createVibe(path);
            setCreating(false);
          }}
        />
      )}
      <ul className="vibe-tree" aria-label="Vibes">
        {tree.map((node) =>
          renaming === node.vibe.id ? (
            <li key={node.vibe.id} style={{ paddingLeft: 8 + node.depth * 14 }}>
              <VibeNameInput
                initial={node.vibe.path}
                label={`New name for ${node.vibe.path}`}
                onCancel={() => setRenaming(null)}
                onSubmit={async (path) => {
                  await renameVibe(node.vibe.id, path);
                  setRenaming(null);
                }}
              />
            </li>
          ) : (
            <VibeRow
              key={node.vibe.id}
              vibe={node.vibe}
              depth={node.depth}
              count={counts.get(node.vibe.id) ?? 0}
              onRename={() => setRenaming(node.vibe.id)}
            />
          ),
        )}
        {tree.length === 0 && !creating && (
          <li className="dim small sidebar-empty">
            No vibes yet. Select tracks and press T.
          </li>
        )}
      </ul>
      <div className="sidebar-bottom">
        <NavItem
          view="sync"
          icon={<ArrowLeftRight size={16} aria-hidden="true" />}
          label="Sync"
        />
        <NavItem
          view="settings"
          icon={<Settings size={16} aria-hidden="true" />}
          label="Settings"
        />
      </div>
    </nav>
  );
}
