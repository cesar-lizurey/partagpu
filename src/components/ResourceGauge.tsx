import { useEffect, useRef, useState } from "react";
import { useT } from "../lib/i18n";

/** A single colored slice inside the gauge bar, used to stack contributors
 *  (you + remote users sharing this machine) on the same gauge. */
export interface GaugeSegment {
  /** Width of the segment in the same unit as `percent` (i.e. the same scale
   *  as the gauge total). The sum of all `value` should not exceed 100. */
  value: number;
  /** Hex string or CSS variable, e.g. `"#6366f1"` or `"var(--color-success)"`. */
  color: string;
  /** Tooltip text shown on hover. */
  name: string;
}

interface ResourceGaugeProps {
  label: string;
  /** Current usage as a 0-100 percent of the gauge. Ignored when `segments`
   *  is provided (the stacked total is the source of truth). */
  percent: number;
  /** Optional textual detail (e.g. "8192 / 16384 Mo"). */
  detail?: string;
  /** When provided, the gauge fill is rendered as a stack of these segments
   *  instead of one solid color. Use this to overlay per-user contributions
   *  (e.g. "you" + each remote user feeding the shared machine). */
  segments?: GaugeSegment[];
  /** Limit value in the same unit as the slider below (0-100 for %, Mo for RAM). */
  limit?: number;
  /** Maximum value the limit can take (100 for %, ramTotalMb for RAM). */
  limitMax?: number;
  /** Step for the limit slider (5 for %, 256 for RAM). */
  limitStep?: number;
  /** Display unit for the limit ("%", "Mo"). */
  limitUnit?: string;
  /** Callback when the user drags the limit cursor. Debounced 300ms upstream. */
  onLimitChange?: (newLimit: number) => void;
  /** Disables the limit interaction (greyed out). */
  limitDisabled?: boolean;
}

export function ResourceGauge({
  label,
  percent,
  detail,
  segments,
  limit,
  limitMax = 100,
  limitStep = 5,
  limitUnit = "%",
  onLimitChange,
  limitDisabled = false,
}: ResourceGaugeProps) {
  const t = useT();
  const clampedPercent = Math.min(100, Math.max(0, percent));
  const color =
    clampedPercent > 80
      ? "var(--color-danger)"
      : clampedPercent > 50
        ? "var(--color-warning)"
        : "var(--color-success)";

  // En mode segments empilés, on ignore `percent` pour le rendu de la barre
  // et on additionne les segments. Le badge en haut continue d'afficher le
  // total parce que `percent` reste le total système (utile quand un
  // process hors PartaGPU consomme de la place qu'on n'attribue à aucun
  // segment).
  const hasSegments = !!segments && segments.length > 0;

  // Local state for instant visual feedback while dragging the limit cursor.
  // The actual cgroup write is debounced through onLimitChange upstream.
  const [localLimit, setLocalLimit] = useState(limit ?? 0);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (limit !== undefined) setLocalLimit(limit);
  }, [limit]);

  const handleChange = (newVal: number) => {
    setLocalLimit(newVal);
    if (!onLimitChange) return;
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => onLimitChange(newVal), 300);
  };

  const limitPercentForDisplay =
    limit !== undefined && limitMax > 0
      ? (localLimit / limitMax) * 100
      : undefined;

  // Format the limit for the badge under the label
  const formattedLimit =
    limit !== undefined
      ? limitUnit === "Mo"
        ? localLimit > 0
          ? `${localLimit} Mo`
          : t("common.unlimited")
        : `${localLimit}${limitUnit}`
      : null;

  return (
    <div className="resource-gauge">
      <div className="resource-gauge__header">
        <span className="resource-gauge__label">{label}</span>
        <span className="resource-gauge__value">
          {clampedPercent.toFixed(0)}%
          {detail && <span className="resource-gauge__detail"> ({detail})</span>}
        </span>
      </div>
      <div
        className={`resource-gauge__track${
          onLimitChange ? " resource-gauge__track--interactive" : ""
        }`}
      >
        {hasSegments ? (
          <div className="resource-gauge__stack" aria-hidden="true">
            {segments!.map((seg, i) => {
              const w = Math.min(100, Math.max(0, seg.value));
              if (w < 0.5) return null;
              return (
                <div
                  key={`${seg.name}-${i}`}
                  className="resource-gauge__segment"
                  style={{ width: `${w}%`, backgroundColor: seg.color }}
                  title={`${seg.name} : ${seg.value.toFixed(1)}${limitUnit === "Mo" ? " Mo" : "%"}`}
                />
              );
            })}
          </div>
        ) : (
          <div
            className="resource-gauge__fill"
            style={{ width: `${clampedPercent}%`, backgroundColor: color }}
          />
        )}
        {limitPercentForDisplay !== undefined && limitPercentForDisplay < 100 && (
          <div
            className="resource-gauge__limit"
            style={{ left: `${limitPercentForDisplay}%` }}
            aria-hidden="true"
          />
        )}
        {onLimitChange && limit !== undefined && (
          <input
            type="range"
            className="resource-gauge__limit-input"
            min={0}
            max={limitMax}
            step={limitStep}
            value={localLimit}
            onChange={(e) => handleChange(Number(e.target.value))}
            disabled={limitDisabled}
            aria-label={t("gauge.input_aria", { label })}
            title={
              limitDisabled
                ? t("gauge.input_disabled_title")
                : t("gauge.input_drag_title")
            }
          />
        )}
      </div>
      {formattedLimit && (
        <div className="resource-gauge__limit-label">
          {t("gauge.share_limit")}
          <strong>{formattedLimit}</strong>
        </div>
      )}
    </div>
  );
}
