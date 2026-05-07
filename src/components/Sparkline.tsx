interface SparklineProps {
  /** Data points oldest → newest. */
  values: number[];
  /** Upper bound of the value axis. Caller's responsibility (e.g. 100 for %). */
  max: number;
  /** Inline width in CSS units. Defaults to 100% of parent. */
  width?: number | string;
  /** Inline height in pixels. */
  height?: number;
  /** Stroke color of the curve. */
  color?: string;
  /** Optional fill color for the area below the curve (alpha-blended). */
  fillColor?: string;
  /** Optional ARIA label for accessibility. */
  ariaLabel?: string;
}

/** Pure-SVG sparkline. No chart library to keep frontend deps minimal.
 *  Renders an open path of `values` normalized over `max`, plus an
 *  optional area fill below the curve. Empty data → renders an empty box. */
export function Sparkline({
  values,
  max,
  width = "100%",
  height = 40,
  color = "var(--color-primary, #6366f1)",
  fillColor,
  ariaLabel,
}: SparklineProps) {
  if (values.length === 0) {
    return (
      <svg
        viewBox="0 0 100 40"
        width={width}
        height={height}
        preserveAspectRatio="none"
        aria-label={ariaLabel}
      />
    );
  }

  const w = 100; // viewBox width — actual rendered width comes from `width` prop
  const h = height;
  const safeMax = max > 0 ? max : 1;
  const stepX = values.length > 1 ? w / (values.length - 1) : 0;

  const points = values.map((v, i) => {
    const x = i * stepX;
    const norm = Math.min(1, Math.max(0, v / safeMax));
    const y = h - norm * h;
    return [x, y] as const;
  });

  const linePath = points
    .map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(2)} ${y.toFixed(2)}`)
    .join(" ");

  const areaPath = fillColor
    ? `${linePath} L${w.toFixed(2)} ${h} L0 ${h} Z`
    : null;

  return (
    <svg
      viewBox={`0 0 ${w} ${h}`}
      width={width}
      height={height}
      preserveAspectRatio="none"
      aria-label={ariaLabel}
      style={{ display: "block" }}
    >
      {areaPath && <path d={areaPath} fill={fillColor} stroke="none" />}
      <path d={linePath} fill="none" stroke={color} strokeWidth={1.5} />
    </svg>
  );
}
