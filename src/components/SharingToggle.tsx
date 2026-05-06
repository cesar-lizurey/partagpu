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

const DISABLE_CONFIRM_MESSAGE =
  "Désactiver le partage va NETTOYER COMPLÈTEMENT PartaGPU sur cette machine :\n" +
  "\n" +
  "  • Le compte système 'partagpu' est supprimé\n" +
  "  • Les tâches en cours sur ce poste sont tuées\n" +
  "  • Le venv géré (torch + numpy, ~2 Go) est supprimé\n" +
  "  • Le cgroup et les règles SSH/sudo sont nettoyés\n" +
  "  • Le pare-feu PartaGPU est fermé\n" +
  "\n" +
  "Pour ré-utiliser PartaGPU ensuite, il faudra tout re-créer (mot de\n" +
  "passe administrateur + ré-installer le venv ~5 min).\n" +
  "\n" +
  "Pour un arrêt temporaire, utilisez plutôt « Pause ».\n" +
  "\n" +
  "Confirmer la désactivation complète ?";

export function SharingToggle({
  status,
  onEnable,
  onDisable,
  onPause,
  onResume,
}: SharingToggleProps) {
  const { label, className } = STATUS_LABELS[status];

  const handleDisable = () => {
    if (window.confirm(DISABLE_CONFIRM_MESSAGE)) {
      onDisable();
    }
  };

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
            <button
              className="btn btn--warning"
              onClick={onPause}
              title="Suspend temporairement les tâches reçues sans rien désinstaller. Cliquez « Reprendre » pour redémarrer instantanément."
            >
              Pause
            </button>
            <button
              className="btn btn--danger"
              onClick={handleDisable}
              title="Nettoie complètement PartaGPU : supprime le compte partagpu, tue les tâches, vire le venv géré, ferme le pare-feu. À utiliser pour libérer la machine après usage."
            >
              Désactiver
            </button>
          </>
        )}
        {status === "Paused" && (
          <>
            <button className="btn btn--primary" onClick={onResume}>
              Reprendre
            </button>
            <button
              className="btn btn--danger"
              onClick={handleDisable}
              title="Nettoie complètement PartaGPU : supprime le compte partagpu, vire le venv géré, ferme le pare-feu."
            >
              Désactiver
            </button>
          </>
        )}
      </div>
    </div>
  );
}
