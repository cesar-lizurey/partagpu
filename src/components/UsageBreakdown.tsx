import type { Task } from "../lib/api";

interface UsageBreakdownProps {
  tasks: Task[];
  totalCpuPercent: number;
  totalRamMb: number;
  totalGpuPercent: number;
  gpuAvailable: boolean;
}

interface UserUsage {
  name: string;
  color: string;
  cpu: number;
  ramMb: number;
  gpu: number;
  taskCount: number;
}

const COLORS = [
  "#6366f1", // indigo
  "#f59e0b", // amber
  "#22c55e", // green
  "#ef4444", // red
  "#06b6d4", // cyan
  "#a855f7", // purple
  "#ec4899", // pink
  "#14b8a6", // teal
];

function aggregateByUser(tasks: Task[]): UserUsage[] {
  const running = tasks.filter((t) => t.status === "Running");
  const map = new Map<string, UserUsage>();

  running.forEach((task) => {
    const key = task.source_machine || task.source_user || "inconnu";
    const existing = map.get(key);
    if (existing) {
      existing.cpu += task.cpu_usage;
      existing.ramMb += task.ram_usage_mb;
      existing.gpu += task.gpu_usage;
      existing.taskCount += 1;
    } else {
      map.set(key, {
        name: key,
        color: COLORS[map.size % COLORS.length],
        cpu: task.cpu_usage,
        ramMb: task.ram_usage_mb,
        gpu: task.gpu_usage,
        taskCount: 1,
      });
    }
  });

  return Array.from(map.values()).sort((a, b) => b.cpu - a.cpu);
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
  const users = aggregateByUser(tasks);

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
            <span className="usage-breakdown__task-count">
              ({u.taskCount} tâche{u.taskCount > 1 ? "s" : ""})
            </span>
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
