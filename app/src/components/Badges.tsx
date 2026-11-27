import { memo } from "react";
import { REVIEW_LABEL, VERDICT_LABEL } from "../lib/format";
import type { QualityReport, Vibe, VibeId } from "../lib/types";

/** Colored quality verdict, with a mark when the DJ has reviewed it. */
export const VerdictBadge = memo(function VerdictBadge({
  quality,
}: {
  quality: QualityReport | null;
}) {
  const verdict = quality?.verdict ?? "unchecked";
  const review = quality?.review ?? "unreviewed";
  const reviewed = review !== "unreviewed";
  const label = reviewed
    ? `${VERDICT_LABEL[verdict]}, ${REVIEW_LABEL[review]}`
    : VERDICT_LABEL[verdict];
  return (
    <span
      className={`badge verdict-${verdict} review-${review}`}
      title={quality?.reason ?? label}
    >
      <span className="badge-dot" aria-hidden="true" />
      {VERDICT_LABEL[verdict]}
      {review === "verified_ok" && <span className="badge-review"> · OK</span>}
      {review === "replace_wanted" && (
        <span className="badge-review"> · Replace</span>
      )}
    </span>
  );
});

export const VibePills = memo(function VibePills({
  ids,
  vibes,
}: {
  ids: readonly VibeId[];
  vibes: ReadonlyMap<VibeId, Vibe>;
}) {
  if (ids.length === 0) return null;
  const paths = ids
    .map((id) => vibes.get(id)?.path)
    .filter((path): path is string => Boolean(path))
    .sort();
  return (
    <span className="pills" title={paths.join(", ")}>
      {paths.map((path) => (
        <span key={path} className="pill">
          {path}
        </span>
      ))}
    </span>
  );
});
