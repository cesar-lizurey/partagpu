import type { Task } from "../lib/api";

interface TaskListProps {
  tasks: Task[];
  direction: "incoming" | "outgoing";
}

const STATUS_LABELS: Record<string, { label: string; className: string }> = {
  Queued: { label: "En attente", className: "badge--queued" },
  Running: { label: "En cours", className: "badge--running" },
  Completed: { label: "Terminée", className: "badge--completed" },
  Failed: { label: "Échouée", className: "badge--failed" },
  Cancelled: { label: "Annulée", className: "badge--disabled" },
};

export function TaskList({ tasks, direction }: TaskListProps) {
  if (tasks.length === 0) {
    return (
      <p className="empty-state">
        {direction === "incoming"
          ? "Personne n'utilise vos ressources actuellement."
          : "Vous n'utilisez aucune ressource distante."}
      </p>
    );
  }

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
        </tr>
      </thead>
      <tbody>
        {tasks.map((task) => {
          const statusInfo = STATUS_LABELS[task.status] ?? {
            label: task.status,
            className: "",
          };
          return (
            <tr key={task.id}>
              <td className="task-command">{task.command}</td>
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
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
