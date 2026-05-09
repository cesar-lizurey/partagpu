import { useEffect, useMemo, useState } from "react";
import {
  cancelIncomingTask,
  cancelOutgoingTask,
  removeIncomingTask,
  removeOutgoingTask,
  type Task,
} from "../lib/api";
import { useT } from "../lib/i18n";
import type { MessageKey } from "../lib/messages";

/** "1m 23s", "12s", "2h 5m". `secs` doit être >= 0. Garde une granularité
 *  utile sans bruit (on ne mélange pas h/m/s à trois niveaux). */
function formatDuration(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const remS = s % 60;
  if (m < 60) return `${m}m ${remS}s`;
  const h = Math.floor(m / 60);
  const remM = m % 60;
  return `${h}h ${remM}m`;
}

/** Durée à afficher pour une tâche : si elle tourne encore, "now − created_at"
 *  rafraîchi par le tick parent ; si elle est terminée, "ended_at − created_at"
 *  (figé). Renvoie null pour les tâches en queue (pas encore démarrées). */
function taskDurationSecs(task: Task, nowSecs: number): number | null {
  if (task.status === "Queued") return null;
  const start = task.created_at;
  const end =
    task.status === "Running"
      ? nowSecs
      : (task.ended_at ?? nowSecs);
  return Math.max(0, end - start);
}

interface TaskListProps {
  tasks: Task[];
  direction: "incoming" | "outgoing";
  /** Called after a successful cancel so the parent can refresh the list. */
  onCancelled?: () => void;
}

const STATUS_INFO: Record<string, { key: MessageKey; className: string }> = {
  Queued: { key: "task.status_queued", className: "badge--queued" },
  Running: { key: "task.status_running", className: "badge--running" },
  Completed: { key: "task.status_completed", className: "badge--completed" },
  Failed: { key: "task.status_failed", className: "badge--failed" },
  Cancelled: { key: "task.status_cancelled", className: "badge--disabled" },
};

const CANCELLABLE = new Set(["Queued", "Running"]);
const REMOVABLE = new Set(["Completed", "Failed", "Cancelled"]);

export function TaskList({ tasks, direction, onCancelled }: TaskListProps) {
  const t = useT();
  const [cancellingId, setCancellingId] = useState<string | null>(null);
  const [removingId, setRemovingId] = useState<string | null>(null);

  // Newest task first. Falls back to id for the rare case where two tasks
  // share the same created_at second so the order stays stable.
  const sortedTasks = useMemo(
    () =>
      [...tasks].sort((a, b) => {
        if (b.created_at !== a.created_at) return b.created_at - a.created_at;
        return b.id.localeCompare(a.id);
      }),
    [tasks],
  );

  // Tick chaque seconde uniquement quand au moins une tâche tourne, sinon
  // pas de re-render inutile. La durée des tâches terminées est figée et
  // n'a pas besoin du ticker.
  const hasRunning = sortedTasks.some((task) => task.status === "Running");
  const [nowSecs, setNowSecs] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    if (!hasRunning) return;
    const id = setInterval(() => {
      setNowSecs(Math.floor(Date.now() / 1000));
    }, 1000);
    return () => clearInterval(id);
  }, [hasRunning]);

  if (sortedTasks.length === 0) {
    return (
      <p className="empty-state">
        {direction === "incoming"
          ? t("task.empty_incoming")
          : t("task.empty_outgoing")}
      </p>
    );
  }

  const handleCancel = async (taskId: string) => {
    setCancellingId(taskId);
    try {
      if (direction === "incoming") {
        await cancelIncomingTask(taskId);
      } else {
        await cancelOutgoingTask(taskId);
      }
      onCancelled?.();
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error("Cancel failed:", err);
      alert(t("task.cancel_failed", { error: String(err) }));
    } finally {
      setCancellingId(null);
    }
  };

  const handleRemove = async (taskId: string) => {
    if (!confirm(t("task.remove_confirm"))) return;
    setRemovingId(taskId);
    try {
      if (direction === "incoming") {
        await removeIncomingTask(taskId);
      } else {
        await removeOutgoingTask(taskId);
      }
      onCancelled?.();
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error("Remove failed:", err);
      alert(t("task.remove_failed", { error: String(err) }));
    } finally {
      setRemovingId(null);
    }
  };

  return (
    <table className="peer-table">
      <thead>
        <tr>
          <th>{t("task.col_command")}</th>
          <th>{direction === "incoming" ? t("task.col_source") : t("task.col_target")}</th>
          <th>{t("task.col_status")}</th>
          <th>{t("task.col_progress")}</th>
          <th>{t("peers.col_cpu")}</th>
          <th>{t("peers.col_ram")}</th>
          <th>{t("peers.col_gpu")}</th>
          <th>{t("task.col_action")}</th>
        </tr>
      </thead>
      <tbody>
        {sortedTasks.map((task) => {
          const info = STATUS_INFO[task.status];
          const label = info ? t(info.key) : task.status;
          const className = info?.className ?? "";
          const canCancel = CANCELLABLE.has(task.status);
          const canRemove = REMOVABLE.has(task.status);
          const isCancelling = cancellingId === task.id;
          const isRemoving = removingId === task.id;
          return (
            <tr key={task.id}>
              <td className="task-command">
                {task.command}
                {task.network_enabled ? (
                  <span
                    className="badge badge--running"
                    style={{ marginLeft: 8, fontSize: "0.75em" }}
                    title={t("task.network_badge_title")}
                  >
                    {t("task.network_badge")}
                  </span>
                ) : null}
              </td>
              <td>
                {direction === "incoming"
                  ? task.source_machine
                  : task.target_machine}
              </td>
              <td>
                <span className={`badge ${className}`}>{label}</span>
              </td>
              <td>
                <div className="progress-bar">
                  <div
                    className="progress-bar__fill"
                    style={{ width: `${task.progress}%` }}
                  />
                  <span className="progress-bar__label">
                    {task.progress.toFixed(0)}%
                  </span>
                </div>
                {(() => {
                  const d = taskDurationSecs(task, nowSecs);
                  if (d === null) return null;
                  const isRunning = task.status === "Running";
                  return (
                    <div
                      className="progress-bar__duration"
                      title={
                        isRunning
                          ? t("task.duration_running_title")
                          : t("task.duration_total_title")
                      }
                    >
                      {isRunning ? "↻ " : "✓ "}
                      {formatDuration(d)}
                    </div>
                  );
                })()}
              </td>
              <td>{task.cpu_usage.toFixed(0)}%</td>
              <td>{task.ram_usage_mb} Mo</td>
              <td>{task.gpu_usage.toFixed(0)}%</td>
              <td>
                <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                  {canCancel ? (
                    <button
                      type="button"
                      onClick={() => handleCancel(task.id)}
                      disabled={isCancelling}
                      title={t("task.cancel_title")}
                    >
                      {isCancelling ? "…" : t("task.cancel_btn")}
                    </button>
                  ) : null}
                  {canRemove ? (
                    <button
                      type="button"
                      className="task-remove-btn"
                      onClick={() => handleRemove(task.id)}
                      disabled={isRemoving}
                      title={t("task.remove_title")}
                      aria-label={t("task.remove_title")}
                    >
                      {isRemoving ? "…" : "🗑"}
                    </button>
                  ) : null}
                  {!canCancel && !canRemove ? (
                    <span style={{ opacity: 0.4 }}>—</span>
                  ) : null}
                </div>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
