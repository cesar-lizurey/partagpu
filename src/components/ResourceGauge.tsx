import { useEffect, useRef, useState } from "react";
import { useT } from "../lib/i18n";

interface ResourceGaugeProps {
  label: string;
  /** Current usage as a 0-100 percent of the gauge. */
  percent: number;
  /** Optional textual detail (e.g. "8192 / 16384 Mo"). */
  detail?: string;
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
        <div
          className="resource-gauge__fill"
          style={{ width: `${clampedPercent}%`, backgroundColor: color }}
        />
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
