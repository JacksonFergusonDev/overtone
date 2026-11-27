import { Check, Minus, Plus } from "lucide-react";
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { errorMessage } from "../lib/backend";
import { fuzzyRank } from "../lib/fuzzy";
import type { TrackId, Vibe } from "../lib/types";
import { parseVibePath, vibeSlug } from "../lib/vibes";
import { selectedTrackIds } from "../store/actions";
import { createVibe, toggleVibe, useLibrary } from "../store/library";
import { notifyError } from "../store/toasts";
import { closeModal, rememberVibe, useUi } from "../store/ui";

type Coverage = "all" | "some" | "none";

type Option =
  | { kind: "vibe"; vibe: Vibe; coverage: Coverage }
  | { kind: "create"; path: string };

const optionId = (index: number) => `quick-tag-option-${index}`;

/** Opened with T or Cmd+K: fuzzy-find a vibe and toggle it on the selection. */
export function QuickTagPalette() {
  // The selection is fixed when the palette opens.
  const [targets] = useState<TrackId[]>(() => selectedTrackIds());
  const vibes = useLibrary((s) => s.vibes);
  const tracks = useLibrary((s) => s.tracks);
  const recent = useUi((s) => s.recentVibes);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    inputRef.current?.focus();
    return () => previous?.focus?.();
  }, []);

  const coverage = useMemo(() => {
    const counts = new Map<number, number>();
    for (const id of targets) {
      for (const vibe of tracks.get(id)?.vibes ?? []) {
        counts.set(vibe, (counts.get(vibe) ?? 0) + 1);
      }
    }
    return (vibe: Vibe): Coverage => {
      const count = counts.get(vibe.id) ?? 0;
      return count === 0 ? "none" : count >= targets.length ? "all" : "some";
    };
  }, [targets, tracks]);

  const parsed = parseVibePath(query);
  const createPath = parsed.ok ? parsed.path : null;
  const options = useMemo<Option[]>(() => {
    let ranked: Vibe[];
    if (query.trim()) {
      ranked = fuzzyRank(query, vibes, (vibe) => vibe.path);
    } else {
      const byId = new Map(vibes.map((vibe) => [vibe.id, vibe]));
      const recents = recent
        .map((id) => byId.get(id))
        .filter((v): v is Vibe => Boolean(v));
      const rest = [...vibes]
        .filter((vibe) => !recent.includes(vibe.id))
        .sort((a, b) => a.path.localeCompare(b.path));
      ranked = [...recents, ...rest];
    }
    const list: Option[] = ranked.map((vibe) => ({
      kind: "vibe",
      vibe,
      coverage: coverage(vibe),
    }));
    if (createPath !== null) {
      const slug = vibeSlug(createPath);
      if (!vibes.some((vibe) => vibeSlug(vibe.path) === slug)) {
        list.push({ kind: "create", path: createPath });
      }
    }
    return list;
  }, [query, vibes, recent, coverage, createPath]);

  const activeIndex = Math.min(active, Math.max(0, options.length - 1));

  const choose = async (option: Option | undefined) => {
    if (!option || busy || targets.length === 0) return;
    setBusy(true);
    try {
      let vibe: Vibe;
      if (option.kind === "create") {
        vibe = await createVibe(option.path);
      } else {
        vibe = option.vibe;
      }
      const assigned = await toggleVibe(targets, vibe.id);
      rememberVibe(vibe.id);
      const count = `${targets.length} ${targets.length === 1 ? "track" : "tracks"}`;
      setMessage(
        assigned
          ? `Added “${vibe.path}” to ${count}`
          : `Removed “${vibe.path}” from ${count}`,
      );
      setQuery("");
      setActive(0);
    } catch (error) {
      // toggleVibe shows its own error; creation errors are shown here.
      if (option.kind === "create")
        notifyError(`Couldn't create the vibe: ${errorMessage(error)}`);
    } finally {
      setBusy(false);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActive((activeIndex + 1) % Math.max(1, options.length));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActive(
        (activeIndex - 1 + options.length) % Math.max(1, options.length),
      );
    } else if (event.key === "Enter") {
      event.preventDefault();
      void choose(options[activeIndex]);
    } else if (event.key === "Escape") {
      event.preventDefault();
      closeModal();
    } else if (event.altKey && /^Digit[1-9]$/.test(event.code)) {
      event.preventDefault();
      const id = recent[Number(event.code.slice(5)) - 1];
      const vibe = vibes.find((v) => v.id === id);
      if (vibe) void choose({ kind: "vibe", vibe, coverage: coverage(vibe) });
    }
  };

  return (
    <div
      className="overlay"
      onMouseDown={(event) =>
        event.target === event.currentTarget && closeModal()
      }
    >
      <div
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label="Quick tag"
      >
        <p className="palette-target">
          Tag {targets.length} selected{" "}
          {targets.length === 1 ? "track" : "tracks"}
        </p>
        <input
          ref={inputRef}
          className="palette-input"
          role="combobox"
          aria-expanded="true"
          aria-controls="quick-tag-list"
          aria-autocomplete="list"
          aria-activedescendant={
            options.length ? optionId(activeIndex) : undefined
          }
          aria-label="Find or create a vibe"
          placeholder="Find or create a vibe, such as Dubs/Deep"
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setActive(0);
            setMessage(null);
          }}
          onKeyDown={onKeyDown}
        />
        {!parsed.ok && query.trim() && (
          <p className="palette-hint error-text">{parsed.error}</p>
        )}
        <ul
          id="quick-tag-list"
          role="listbox"
          className="palette-list"
          aria-label="Vibes"
        >
          {options.map((option, index) => {
            const isActive = index === activeIndex;
            if (option.kind === "create") {
              return (
                <li
                  key="create"
                  id={optionId(index)}
                  role="option"
                  aria-selected={isActive}
                  className={`palette-option${isActive ? " active" : ""}`}
                  onMouseEnter={() => setActive(index)}
                  onClick={() => void choose(option)}
                >
                  <Plus size={14} aria-hidden="true" />
                  <span>Create vibe “{option.path}”</span>
                </li>
              );
            }
            const recentIndex = recent.indexOf(option.vibe.id);
            return (
              <li
                key={option.vibe.id}
                id={optionId(index)}
                role="option"
                aria-selected={isActive}
                aria-checked={
                  option.coverage === "all"
                    ? true
                    : option.coverage === "some"
                      ? "mixed"
                      : false
                }
                className={`palette-option${isActive ? " active" : ""}`}
                onMouseEnter={() => setActive(index)}
                onClick={() => void choose(option)}
              >
                <span
                  className={`coverage coverage-${option.coverage}`}
                  aria-hidden="true"
                >
                  {option.coverage === "all" ? (
                    <Check size={14} />
                  ) : option.coverage === "some" ? (
                    <Minus size={14} />
                  ) : null}
                </span>
                <span className="truncate" title={option.vibe.path}>
                  {option.vibe.path}
                </span>
                <span className="palette-meta">
                  {option.coverage === "all"
                    ? "All selected"
                    : option.coverage === "some"
                      ? "Some selected"
                      : ""}
                  {!query.trim() && recentIndex >= 0 && recentIndex < 9 && (
                    <kbd>⌥{recentIndex + 1}</kbd>
                  )}
                </span>
              </li>
            );
          })}
          {options.length === 0 && (
            <li className="palette-empty">
              No vibes yet. Type a name to create one.
            </li>
          )}
        </ul>
        <p className="palette-status" role="status">
          {message ?? "Enter toggles the highlighted vibe. Esc closes."}
        </p>
      </div>
    </div>
  );
}
