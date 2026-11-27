import { useEffect, useMemo, useRef, useState, type PointerEvent } from "react";
import { formatClock } from "../lib/format";
import type { TrackId } from "../lib/types";
import { canvasSize, resamplePeaks, timeToX, xToTime } from "../lib/waveform";
import { loadWaveform, useLibrary } from "../store/library";
import { audioElement, seek, usePlayer } from "../store/player";

const BAR = 2;
const GAP = 1;

function cssVar(name: string, fallback: string): string {
  if (typeof getComputedStyle !== "function") return fallback;
  const value = getComputedStyle(document.documentElement)
    .getPropertyValue(name)
    .trim();
  return value || fallback;
}

function draw(
  canvas: HTMLCanvasElement,
  columns: [number, number][] | null,
  cssWidth: number,
  cssHeight: number,
  playedX: number,
): void {
  const context = canvas.getContext("2d");
  if (!context) return;
  const ratio = canvas.width / Math.max(1, cssWidth);
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.clearRect(0, 0, cssWidth, cssHeight);
  const played = cssVar("--wave-played", "#38bdf8");
  const rest = cssVar("--wave", "#475569");
  const mid = cssHeight / 2;
  if (!columns) {
    // No peaks yet: a flat line still shows progress.
    context.fillStyle = rest;
    context.fillRect(0, mid - 0.5, cssWidth, 1);
    context.fillStyle = played;
    context.fillRect(0, mid - 0.5, playedX, 1);
  } else {
    columns.forEach(([min, max], index) => {
      const x = index * (BAR + GAP);
      const top = mid - Math.max(max, 0.02) * mid;
      const bottom = mid - Math.min(min, -0.02) * mid;
      context.fillStyle = x + BAR <= playedX ? played : rest;
      context.fillRect(x, top, BAR, Math.max(1, bottom - top));
    });
  }
  context.fillStyle = cssVar("--text", "#e5e7eb");
  context.fillRect(Math.min(cssWidth - 1, playedX), 0, 1, cssHeight);
}

/** Canvas scrubber: click or drag to seek; the played part is tinted. */
export function Waveform({ trackId }: { trackId: TrackId | null }) {
  const peaks = useLibrary((s) =>
    trackId === null ? undefined : s.waveforms.get(trackId),
  );
  const playing = usePlayer((s) => s.playing);
  const duration = usePlayer((s) => s.duration);
  const position = usePlayer((s) => s.position);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const dragging = useRef(false);
  const [size, setSize] = useState({ width: 0, height: 0 });

  useEffect(() => {
    if (trackId !== null) void loadWaveform(trackId);
  }, [trackId]);

  useEffect(() => {
    const element = wrapRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (rect) setSize({ width: rect.width, height: rect.height });
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const columns = useMemo(
    () =>
      peaks && size.width > 0
        ? resamplePeaks(peaks.buckets, Math.floor(size.width / (BAR + GAP)))
        : null,
    [peaks, size.width],
  );

  // Redraw on resize, new peaks, or seeks; while playing, every frame.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || size.width === 0) return;
    const backing = canvasSize(
      size.width,
      size.height,
      window.devicePixelRatio || 1,
    );
    if (canvas.width !== backing.width) canvas.width = backing.width;
    if (canvas.height !== backing.height) canvas.height = backing.height;
    const render = () => {
      const audio = audioElement();
      const now = audio && trackId !== null ? audio.currentTime : position;
      const length =
        audio && Number.isFinite(audio.duration) ? audio.duration : duration;
      draw(
        canvas,
        columns,
        size.width,
        size.height,
        timeToX(now, length, size.width),
      );
    };
    render();
    if (!playing) return;
    let frame = requestAnimationFrame(function tick() {
      render();
      frame = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(frame);
  }, [columns, size, playing, position, duration, trackId]);

  const seekTo = (event: PointerEvent<HTMLDivElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const audio = audioElement();
    const length =
      audio && Number.isFinite(audio.duration) ? audio.duration : duration;
    seek(xToTime(event.clientX - rect.left, rect.width, length));
  };

  return (
    <div
      ref={wrapRef}
      className={`waveform${trackId === null ? " disabled" : ""}`}
      role="slider"
      aria-label="Seek"
      aria-valuemin={0}
      aria-valuemax={Math.round(duration)}
      aria-valuenow={Math.round(position)}
      aria-valuetext={`${formatClock(position)} of ${formatClock(duration)}`}
      aria-disabled={trackId === null}
      onPointerDown={(event) => {
        if (trackId === null) return;
        dragging.current = true;
        event.currentTarget.setPointerCapture?.(event.pointerId);
        seekTo(event);
      }}
      onPointerMove={(event) => {
        if (dragging.current) seekTo(event);
      }}
      onPointerUp={(event) => {
        dragging.current = false;
        event.currentTarget.releasePointerCapture?.(event.pointerId);
      }}
    >
      <canvas ref={canvasRef} aria-hidden="true" />
    </div>
  );
}
