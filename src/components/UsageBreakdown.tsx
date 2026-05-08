import type { Task } from "../lib/api";
import { useT } from "../lib/i18n";
import { aggregateByUser } from "../lib/usage";

interface UsageBreakdownProps {
  tasks: Task[];
  totalCpuPercent: number;
  totalRamMb: number;
  totalGpuPercent: number;
  gpuAvailable: boolean;
}

function StackedBar({
  segments,
  total,
  unit,
}: {
  segments: { value: number; color: string; name: string }[];
  total: number;
  unit: string;
}) {
  if (total <= 0) return null;
  const used = segments.reduce((s, seg) => s + seg.value, 0);

  return (
    <div className="stacked-bar">
      <div className="stacked-bar__track">
        {segments.map((seg) => {
          const pct = (seg.value / total) * 100;
          if (pct < 0.5) return null;
          return (
            <div
              key={seg.name}
              className="stacked-bar__segment"
              style={{ width: `${pct}%`, backgroundColor: seg.color }}
              title={`${seg.name} : ${seg.value.toFixed(1)}${unit}`}
            />
          );
        })}
      </div>
      <span className="stacked-bar__label">
        {used.toFixed(0)}{unit} / {total}{unit}
      </span>
    </div>
  );
}

export function UsageBreakdown({
  tasks,
  totalCpuPercent,
  totalRamMb,
  totalGpuPercent,
  gpuAvailable,
}: UsageBreakdownProps) {
  const t = useT();
  const users = aggregateByUser(tasks, t("common.unknown"));

  if (users.length === 0) {
    return null;
  }

  return (
    <div className="usage-breakdown">
      <div className="usage-breakdown__legend">
        {users.map((u) => (
          <span key={u.name} className="usage-breakdown__legend-item">
            <span
              className="usage-breakdown__swatch"
              style={{ backgroundColor: u.color }}
            />
            <span>{u.name}</span>
            {u.taskCount !== null && (
              <span className="usage-breakdown__task-count">
                {t(
                  u.taskCount === 1
                    ? "breakdown.tasks_one"
                    : "breakdown.tasks_many",
                  { n: u.taskCount },
                )}
              </span>
            )}
          </span>
        ))}
      </div>

      <div className="usage-breakdown__bars">
        <div className="usage-breakdown__row">
          <span className="usage-breakdown__row-label">CPU</span>
          <StackedBar
            segments={users.map((u) => ({
              value: u.cpu,
              color: u.color,
              name: u.name,
            }))}
            total={totalCpuPercent}
            unit="%"
          />
        </div>

        <div className="usage-breakdown__row">
          <span className="usage-breakdown__row-label">RAM</span>
          <StackedBar
            segments={users.map((u) => ({
              value: u.ramMb,
              color: u.color,
              name: u.name,
            }))}
            total={totalRamMb}
            unit=" Mo"
          />
        </div>

        {gpuAvailable && (
          <div className="usage-breakdown__row">
            <span className="usage-breakdown__row-label">GPU</span>
            <StackedBar
              segments={users.map((u) => ({
                value: u.gpu,
                color: u.color,
                name: u.name,
              }))}
              total={totalGpuPercent}
              unit="%"
            />
          </div>
        )}
      </div>
    </div>
  );
}
