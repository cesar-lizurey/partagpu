import { useEffect, useMemo, useState } from "react";
import { dispatchTask, type Peer, type Task } from "../lib/api";

interface TaskDispatcherProps {
  /** Verified peers that have sharing enabled (the only ones we can target). */
  peers: Peer[];
  /** Optional callback fired after the dispatch resolves. */
  onDispatched?: () => void;
}

const STATUS_LABELS: Record<string, { label: string; className: string }> = {
  Queued: { label: "En attente", className: "badge--queued" },
  Running: { label: "En cours", className: "badge--running" },
  Completed: { label: "Terminée", className: "badge--completed" },
  Failed: { label: "Échouée", className: "badge--failed" },
  Cancelled: { label: "Annulée", className: "badge--disabled" },
};

/** Parse a shell-like command line into argv. Supports single + double quotes
 *  and backslash escapes. Doesn't expand env vars / globs (intentionally). */
function parseCommand(input: string): string[] {
  const args: string[] = [];
  let current = "";
  let inSingle = false;
  let inDouble = false;
  let started = false;

  for (let i = 0; i < input.length; i++) {
    const c = input[i];
    if (inSingle) {
      if (c === "'") inSingle = false;
      else {
        current += c;
        started = true;
      }
    } else if (inDouble) {
      if (c === '"') inDouble = false;
      else if (c === "\\" && i + 1 < input.length) {
        current += input[++i];
        started = true;
      } else {
        current += c;
        started = true;
      }
    } else if (c === "'") {
      inSingle = true;
      started = true;
    } else if (c === '"') {
      inDouble = true;
      started = true;
    } else if (c === "\\" && i + 1 < input.length) {
      current += input[++i];
      started = true;
    } else if (c === " " || c === "\t" || c === "\n") {
      if (started) {
        args.push(current);
        current = "";
        started = false;
      }
    } else {
      current += c;
      started = true;
    }
  }
  if (started) args.push(current);
  return args;
}

export function TaskDispatcher({ peers, onDispatched }: TaskDispatcherProps) {
  const targets = useMemo(
    () => peers.filter((p) => p.verified && p.sharing_enabled),
    [peers],
  );

  const [selectedIp, setSelectedIp] = useState<string>("");
  const [commandInput, setCommandInput] = useState<string>(
    'python3 -c "import socket; print(socket.gethostname())"',
  );
  const [networkEnabled, setNetworkEnabled] = useState(false);
  const [timeoutSecs, setTimeoutSecs] = useState(60);
  const [isLaunching, setIsLaunching] = useState(false);
  const [result, setResult] = useState<Task | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Auto-select first target when the list changes
  useEffect(() => {
    if (!selectedIp && targets.length > 0) {
      setSelectedIp(targets[0].ip);
    } else if (selectedIp && !targets.find((t) => t.ip === selectedIp)) {
      // selected peer disappeared
      setSelectedIp(targets[0]?.ip ?? "");
    }
  }, [targets, selectedIp]);

  const parsedArgs = useMemo(() => parseCommand(commandInput), [commandInput]);

  const handleLaunch = async () => {
    if (!selectedIp) {
      setError("Aucun pair sélectionné.");
      return;
    }
    if (parsedArgs.length === 0) {
      setError("La commande est vide.");
      return;
    }
    setError(null);
    setIsLaunching(true);
    setResult(null);
    try {
      const task = await dispatchTask(selectedIp, parsedArgs, {
        timeoutSecs,
        network: networkEnabled,
      });
      setResult(task);
      onDispatched?.();
    } catch (e) {
      setError(String(e));
    } finally {
      setIsLaunching(false);
    }
  };

  if (targets.length === 0) {
    return (
      <div className="task-dispatcher">
        <p className="empty-state">
          Aucun pair vérifié ne partage de ressources actuellement. Activez
          le partage côté camarade et vérifiez que vous êtes dans la même
          salle.
        </p>
      </div>
    );
  }

  const statusInfo = result
    ? STATUS_LABELS[result.status] ?? { label: result.status, className: "" }
    : null;

  return (
    <div className="task-dispatcher">
      <div className="task-dispatcher__form">
        <div className="task-dispatcher__row">
          <label className="task-dispatcher__field">
            <span className="task-dispatcher__label">Pair cible</span>
            <select
              value={selectedIp}
              onChange={(e) => setSelectedIp(e.target.value)}
              disabled={isLaunching}
              className="task-dispatcher__input"
            >
              {targets.map((p) => (
                <option key={p.id} value={p.ip}>
                  {p.display_name || p.hostname} ({p.ip}) — {p.gpu_count ?? 1} GPU
                </option>
              ))}
            </select>
          </label>

          <label className="task-dispatcher__field task-dispatcher__field--narrow">
            <span className="task-dispatcher__label">Timeout (s)</span>
            <input
              type="number"
              min={5}
              max={86400}
              value={timeoutSecs}
              onChange={(e) => setTimeoutSecs(Number(e.target.value))}
              disabled={isLaunching}
              className="task-dispatcher__input"
            />
          </label>
        </div>

        <label className="task-dispatcher__field">
          <span className="task-dispatcher__label">Commande</span>
          <input
            type="text"
            value={commandInput}
            onChange={(e) => setCommandInput(e.target.value)}
            disabled={isLaunching}
            placeholder='python3 -c "print(42)"'
            spellCheck={false}
            autoComplete="off"
            className="task-dispatcher__input task-dispatcher__input--mono"
          />
          {parsedArgs.length > 0 ? (
            <small className="task-dispatcher__parsed">
              argv : <code>{JSON.stringify(parsedArgs)}</code>
            </small>
          ) : null}
        </label>

        <div className="task-dispatcher__network">
          <label className="task-dispatcher__checkbox">
            <input
              type="checkbox"
              checked={networkEnabled}
              onChange={(e) => setNetworkEnabled(e.target.checked)}
              disabled={isLaunching}
            />
            <span>Autoriser l'accès réseau dans le sandbox du pair</span>
          </label>
          <p className="task-dispatcher__help">
            Par défaut, la tâche tourne sans accès réseau (isolation maximale).
            Cochez cette case si votre commande a besoin de :{" "}
            <strong>télécharger des données</strong> (HTTP, HuggingFace…),
            joindre un autre service du LAN, ou faire du{" "}
            <strong>DDP / NCCL</strong> (rendezvous entre rangs).
          </p>
        </div>

        <div className="task-dispatcher__actions">
          <button
            type="button"
            onClick={handleLaunch}
            disabled={isLaunching || !selectedIp || parsedArgs.length === 0}
            className="btn btn--primary"
          >
            {isLaunching ? "Exécution..." : "Lancer"}
          </button>
        </div>
      </div>

      {error ? <div className="alert alert--error">{error}</div> : null}

      {result ? (
        <div className="task-dispatcher__result">
          <div className="task-dispatcher__result-header">
            <span className={`badge ${statusInfo!.className}`}>
              {statusInfo!.label}
            </span>
            <span style={{ marginLeft: 12, opacity: 0.7 }}>
              cible : <strong>{result.target_machine}</strong>
              {" · "}
              exit code :{" "}
              <strong>{result.exit_code ?? "—"}</strong>
            </span>
          </div>
          {result.output ? (
            <details open>
              <summary>stdout ({result.output.length} car.)</summary>
              <pre className="task-dispatcher__pre">{result.output}</pre>
            </details>
          ) : null}
          {result.error_output ? (
            <details open={!result.output}>
              <summary>stderr ({result.error_output.length} car.)</summary>
              <pre className="task-dispatcher__pre task-dispatcher__pre--err">
                {result.error_output}
              </pre>
            </details>
          ) : null}
          {!result.output && !result.error_output ? (
            <p style={{ opacity: 0.6, fontStyle: "italic" }}>
              (aucune sortie)
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
