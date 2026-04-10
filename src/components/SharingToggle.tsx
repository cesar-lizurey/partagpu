import type { SharingStatus } from "../lib/api";

interface SharingToggleProps {
  status: SharingStatus;
  onEnable: () => void;
  onDisable: () => void;
  onPause: () => void;
  onResume: () => void;
}

const STATUS_LABELS: Record<SharingStatus, { label: string; className: string }> = {
  Disabled: { label: "Désactivé", className: "status--disabled" },
  Active: { label: "Actif", className: "status--active" },
  Paused: { label: "En pause", className: "status--paused" },
};

export function SharingToggle({
  status,
  onEnable,
  onDisable,
  onPause,
  onResume,
}: SharingToggleProps) {
  const { label, className } = STATUS_LABELS[status];

  return (
    <div className="sharing-toggle">
      <div className={`sharing-toggle__status ${className}`}>
        <span className="sharing-toggle__dot" />
        <span>{label}</span>
      </div>
      <div className="sharing-toggle__actions">
        {status === "Disabled" && (
          <button className="btn btn--primary" onClick={onEnable}>
            Activer le partage
          </button>
        )}
        {status === "Active" && (
          <>
            <button className="btn btn--warning" onClick={onPause}>
              Pause
            </button>
            <button className="btn btn--danger" onClick={onDisable}>
              Désactiver
            </button>
          </>
        )}
        {status === "Paused" && (
          <>
            <button className="btn btn--primary" onClick={onResume}>
              Reprendre
            </button>
            <button className="btn btn--danger" onClick={onDisable}>
              Désactiver
            </button>
          </>
        )}
      </div>
    </div>
  );
}
