import { useEffect, useState } from "react";
import {
  getManagedVenvStatus,
  removeManagedVenv,
  setupManagedVenv,
  type ManagedVenvStatus,
} from "../lib/api";

/**
 * Panel for the managed Python venv (provides torch + numpy to the sandbox so
 * peers don't have to `sudo pip install --break-system-packages` themselves).
 */
export function ManagedVenvPanel() {
  const [status, setStatus] = useState<ManagedVenvStatus | null>(null);
  const [busy, setBusy] = useState<"install" | "remove" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    try {
      setStatus(await getManagedVenvStatus());
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const handleInstall = async () => {
    if (
      !confirm(
        "Installer la toolkit ML dans le venv géré ?\n\n" +
          "Packages : torch, torchvision, numpy, scipy, pandas, scikit-learn, matplotlib, pillow.\n" +
          "Téléchargement de ~3 Go, prend 5 à 10 minutes selon votre connexion.\n" +
          "Le mot de passe administrateur sera demandé.",
      )
    ) {
      return;
    }
    setError(null);
    setBusy("install");
    try {
      await setupManagedVenv();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const handleRemove = async () => {
    if (
      !confirm(
        "Supprimer le venv géré ?\n\n" +
          "Les tâches reçues qui utilisaient torch/numpy via ce venv " +
          "échoueront jusqu'à réinstallation.",
      )
    ) {
      return;
    }
    setError(null);
    setBusy("remove");
    try {
      await removeManagedVenv();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  if (!status) {
    return <p className="empty-state">Chargement…</p>;
  }

  return (
    <div className="managed-venv">
      <p className="managed-venv__intro">
        Pour que les tâches Python reçues puissent <code>import torch</code>{" "}
        (et tout l'écosystème data science classique) sans que vous ayez à{" "}
        <code>sudo pip install</code>, PartaGPU peut provisionner un venv
        Python à <code>{status.path}</code> avec une <strong>toolkit ML
        complète</strong> : <code>torch</code>, <code>torchvision</code>,{" "}
        <code>numpy</code>, <code>scipy</code>, <code>pandas</code>,{" "}
        <code>scikit-learn</code>, <code>matplotlib</code>,{" "}
        <code>pillow</code>. Le sandbox bind ce venv automatiquement et fait
        pointer <code>python3</code> dessus.
      </p>

      <div className="managed-venv__status">
        {status.installed ? (
          <>
            <span className="badge badge--completed">Installé</span>
            <span className="managed-venv__path">
              <code>{status.path}</code>
            </span>
          </>
        ) : (
          <>
            <span className="badge badge--disabled">Non installé</span>
            <span className="managed-venv__hint">
              Sans ça, les pairs doivent installer torch + ses dépendances
              manuellement (<code>sudo pip install --break-system-packages …</code>).
            </span>
          </>
        )}
      </div>

      <div className="managed-venv__actions">
        {status.installed ? (
          <>
            <button
              type="button"
              onClick={handleInstall}
              disabled={busy !== null}
              className="btn btn--secondary"
              title="Réinstalle / met à jour torch + numpy"
            >
              {busy === "install" ? "Mise à jour…" : "Mettre à jour"}
            </button>
            <button
              type="button"
              onClick={handleRemove}
              disabled={busy !== null}
              className="btn btn--danger"
            >
              {busy === "remove" ? "Suppression…" : "Supprimer"}
            </button>
          </>
        ) : (
          <button
            type="button"
            onClick={handleInstall}
            disabled={busy !== null}
            className="btn btn--primary"
          >
            {busy === "install"
              ? "Installation… (5 à 10 min)"
              : "Installer la toolkit ML (~3 Go)"}
          </button>
        )}
      </div>

      {busy === "install" ? (
        <p className="managed-venv__progress">
          Le téléchargement et l'installation tournent en arrière-plan via
          pkexec — laissez la fenêtre ouverte. Pour suivre la progression,
          regardez le terminal d'où vous avez lancé l'app.
        </p>
      ) : null}

      {error ? <div className="alert alert--error">{error}</div> : null}
    </div>
  );
}
