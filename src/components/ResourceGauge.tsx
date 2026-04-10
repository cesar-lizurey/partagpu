interface ResourceGaugeProps {
  label: string;
  percent: number;
  detail?: string;
  limit?: number;
}

export function ResourceGauge({
  label,
  percent,
  detail,
  limit,
}: ResourceGaugeProps) {
  const clampedPercent = Math.min(100, Math.max(0, percent));
  const color =
    clampedPercent > 80
      ? "var(--color-danger)"
      : clampedPercent > 50
        ? "var(--color-warning)"
        : "var(--color-success)";

  return (
    <div className="resource-gauge">
      <div className="resource-gauge__header">
        <span className="resource-gauge__label">{label}</span>
        <span className="resource-gauge__value">
          {clampedPercent.toFixed(0)}%
          {detail && <span className="resource-gauge__detail"> ({detail})</span>}
        </span>
      </div>
      <div className="resource-gauge__track">
        <div
          className="resource-gauge__fill"
          style={{ width: `${clampedPercent}%`, backgroundColor: color }}
        />
        {limit !== undefined && limit < 100 && (
          <div
            className="resource-gauge__limit"
            style={{ left: `${limit}%` }}
            title={`Limite : ${limit}%`}
          />
        )}
      </div>
    </div>
  );
}
