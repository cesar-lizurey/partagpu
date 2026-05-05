import { useState } from "react";
import {
  cancelIncomingTask,
  cancelOutgoingTask,
  type Task,
} from "../lib/api";

interface TaskListProps {
  tasks: Task[];
  direction: "incoming" | "outgoing";
  /** Called after a successful cancel so the parent can refresh the list. */
  onCancelled?: () => void;
}

const STATUS_LABELS: Record<string, { label: string; className: string }> = {
  Queued: { label: "En attente", className: "badge--queued" },
  Running: { label: "En cours", className: "badge--running" },
  Completed: { label: "Terminée", className: "badge--completed" },
  Failed: { label: "Échouée", className: "badge--failed" },
  Cancelled: { label: "Annulée", className: "badge--disabled" },
};

const CANCELLABLE = new Set(["Queued", "Running"]);

export function TaskList({ tasks, direction, onCancelled }: TaskListProps) {
  const [cancellingId, setCancellingId] = useState<string | null>(null);

  if (tasks.length === 0) {
    return (
      <p className="empty-state">
        {direction === "incoming"
          ? "Personne n'utilise vos ressources actuellement."
          : "Vous n'utilisez aucune ressource distante."}
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
      alert(`Annulation refusée : ${String(err)}`);
    } finally {
      setCancellingId(null);
    }
  };

  return (
    <table className="peer-table">
      <thead>
        <tr>
          <th>Commande</th>
          <th>{direction === "incoming" ? "Source" : "Cible"}</th>
          <th>Statut</th>
          <th>Progression</th>
          <th>CPU</th>
          <th>RAM</th>
          <th>GPU</th>
          <th>Action</th>
        </tr>
      </thead>
      <tbody>
        {tasks.map((task) => {
          const statusInfo = STATUS_LABELS[task.status] ?? {
            label: task.status,
            className: "",
          };
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
                    title="Sandbox avec accès réseau (DDP rendezvous)"
                  >
                    réseau
                  </span>
                ) : null}
              </td>
              <td>
                {direction === "incoming"
                  ? task.source_machine
                  : task.target_machine}
              </td>
              <td>
                <span className={`badge ${statusInfo.className}`}>
                  {statusInfo.label}
                </span>
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
                    title="Arrêter cette tâche"
                  >
                    {isCancelling ? "…" : "Stop"}
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
