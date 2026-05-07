import { useState } from "react";
import {
  cancelIncomingTask,
  cancelOutgoingTask,
  type Task,
} from "../lib/api";
import { useT } from "../lib/i18n";
import type { MessageKey } from "../lib/messages";

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

export function TaskList({ tasks, direction, onCancelled }: TaskListProps) {
  const t = useT();
  const [cancellingId, setCancellingId] = useState<string | null>(null);

  if (tasks.length === 0) {
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
        {tasks.map((task) => {
          const info = STATUS_INFO[task.status];
          const label = info ? t(info.key) : task.status;
          const className = info?.className ?? "";
          const canCancel = CANCELLABLE.has(task.status);
          const isCancelling = cancellingId === task.id;
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
              </td>
              <td>{task.cpu_usage.toFixed(0)}%</td>
              <td>{task.ram_usage_mb} Mo</td>
              <td>{task.gpu_usage.toFixed(0)}%</td>
              <td>
                {canCancel ? (
                  <button
                    type="button"
                    onClick={() => handleCancel(task.id)}
                    disabled={isCancelling}
                    title={t("task.cancel_title")}
                  >
                    {isCancelling ? "…" : t("task.cancel_btn")}
                  </button>
                ) : (
                  <span style={{ opacity: 0.4 }}>—</span>
                )}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
