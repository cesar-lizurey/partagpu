import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import {
  getManagedVenvStatus,
  removeManagedVenv,
  setupManagedVenv,
  type ManagedVenvStatus,
} from "../lib/api";

/** Maximum number of log lines kept in memory while the helper streams. */
const MAX_LOG_LINES = 500;

/**
 * Panel for the managed Python venv (provides torch + numpy to the sandbox so
 * peers don't have to `sudo pip install --break-system-packages` themselves).
 */
export function ManagedVenvPanel() {
  const [status, setStatus] = useState<ManagedVenvStatus | null>(null);
  const [busy, setBusy] = useState<"install" | "remove" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [installLog, setInstallLog] = useState<string[]>([]);
  const logBoxRef = useRef<HTMLPreElement | null>(null);

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

  // Listen to helper output events (stdout + stderr lines streamed by the
  // backend during long-running installs). Append to the log buffer with
  // a hard cap so a chatty pip install doesn't blow up memory.
  useEffect(() => {
    let unlistenStdout: UnlistenFn | null = null;
    let unlistenStderr: UnlistenFn | null = null;
    (async () => {
      unlistenStdout = await listen<string>("helper-output", (e) => {
        setInstallLog((prev) => {
          const next = [...prev, e.payload];
          return next.length > MAX_LOG_LINES
            ? next.slice(next.length - MAX_LOG_LINES)
            : next;
        });
      });
      unlistenStderr = await listen<string>("helper-output-err", (e) => {
        setInstallLog((prev) => {
          const next = [...prev, `[stderr] ${e.payload}`];
          return next.length > MAX_LOG_LINES
            ? next.slice(next.length - MAX_LOG_LINES)
            : next;
        });
      });
    })();
    return () => {
      unlistenStdout?.();
      unlistenStderr?.();
    };
  }, []);

  // Auto-scroll the log to the bottom as new lines arrive.
  useEffect(() => {
    if (logBoxRef.current) {
      logBoxRef.current.scrollTop = logBoxRef.current.scrollHeight;
    }
  }, [installLog]);

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
    setInstallLog([]);
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
          Téléchargement et installation en cours (5 à 10 minutes selon
          votre connexion). Laissez la fenêtre ouverte — la progression
          de pip s'affiche ci-dessous en temps réel.
        </p>
      ) : null}

      {(busy === "install" || installLog.length > 0) && (
        <details
          className="managed-venv__log-box"
          open={busy === "install"}
        >
          <summary>
            Log d'installation ({installLog.length} ligne
            {installLog.length === 1 ? "" : "s"})
          </summary>
          <pre ref={logBoxRef} className="managed-venv__log">
            {installLog.length > 0
              ? installLog.join("\n")
              : "(en attente de la première ligne…)"}
          </pre>
        </details>
      )}

      {error ? <div className="alert alert--error">{error}</div> : null}
    </div>
  );
}
