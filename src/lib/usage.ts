import type { Task } from "./api";

/** A single contributor in a gauge / breakdown bar. The first entry in any
 *  list is conventionally "you" (local non-PartaGPU usage), the remaining
 *  entries one per remote user feeding the same shared resource. */
export interface UserUsage {
  /** Stable label : displayed in the legend and in segment tooltips. For
   *  remote users this is `source_machine` if set (more informative on a
   *  shared classroom LAN), otherwise `source_user`. */
  name: string;
  /** Hex color assigned by `assignColors` ; consistent across both the
   *  stacked gauge and the standalone breakdown so the user can correlate
   *  visually. */
  color: string;
  cpu: number;
  ramMb: number;
  gpu: number;
  /** How many running tasks this user has — null when this is the local
   *  "you" segment (which doesn't correspond to a PartaGPU task). */
  taskCount: number | null;
}

/** Color used for the "you" segment on the gauges, consistent with the
 *  `--color-success` accent used elsewhere for "your own usage is OK". */
export const SELF_COLOR = "var(--color-success)";

/** Palette assigned to remote users in order of arrival. Picked to be
 *  visually distinct from each other AND from `SELF_COLOR` (no green). */
export const USER_COLORS = [
  "#6366f1", // indigo
  "#f59e0b", // amber
  "#ef4444", // red
  "#06b6d4", // cyan
  "#a855f7", // purple
  "#ec4899", // pink
  "#14b8a6", // teal
  "#fbbf24", // yellow
];

/** Aggregate `Running` tasks by source machine/user. Each unique source
 *  becomes one entry ; multiple tasks from the same source are summed. The
 *  result is sorted by descending CPU so the largest contributor appears
 *  first in the stacked bar. */
export function aggregateByUser(
  tasks: Task[],
  unknownLabel: string,
): UserUsage[] {
  const running = tasks.filter((t) => t.status === "Running");
  const map = new Map<string, UserUsage>();

  running.forEach((task) => {
    const key = task.source_machine || task.source_user || unknownLabel;
    const existing = map.get(key);
    if (existing) {
      existing.cpu += task.cpu_usage;
      existing.ramMb += task.ram_usage_mb;
      existing.gpu += task.gpu_usage;
      existing.taskCount = (existing.taskCount ?? 0) + 1;
    } else {
      map.set(key, {
        name: key,
        color: USER_COLORS[map.size % USER_COLORS.length],
        cpu: task.cpu_usage,
        ramMb: task.ram_usage_mb,
        gpu: task.gpu_usage,
        taskCount: 1,
      });
    }
  });

  return Array.from(map.values()).sort((a, b) => b.cpu - a.cpu);
}

/** Build the local "you" segment by subtracting remote tasks from the system
 *  total. Returns null if local usage rounds to zero (avoids a 0-width sliver
 *  on the bar). */
export function buildSelfUsage(
  totalCpuPercent: number,
  totalRamMb: number,
  totalGpuPercent: number,
  remoteUsers: UserUsage[],
  selfLabel: string,
): UserUsage | null {
  const remoteCpu = remoteUsers.reduce((s, u) => s + u.cpu, 0);
  const remoteRam = remoteUsers.reduce((s, u) => s + u.ramMb, 0);
  const remoteGpu = remoteUsers.reduce((s, u) => s + u.gpu, 0);
  const cpu = Math.max(0, totalCpuPercent - remoteCpu);
  const ramMb = Math.max(0, totalRamMb - remoteRam);
  const gpu = Math.max(0, totalGpuPercent - remoteGpu);
  if (cpu < 0.5 && ramMb < 1 && gpu < 0.5) {
    return null;
  }
  return {
    name: selfLabel,
    color: SELF_COLOR,
    cpu,
    ramMb,
    gpu,
    taskCount: null,
  };
}
