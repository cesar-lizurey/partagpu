import { useEffect, useMemo, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  dispatchTask,
  type Peer,
  type Task,
  type WorkspaceFile,
} from "../lib/api";

/** Hard limit enforced by the peer-side sandbox. Sum of file sizes. */
const WORKSPACE_MAX_BYTES = 16 * 1024 * 1024;

async function fileToBase64(file: File): Promise<string> {
  const buf = await file.arrayBuffer();
  const bytes = new Uint8Array(buf);
  // Build the binary string in chunks to avoid stack overflow on large files
  // (String.fromCharCode.apply blows up around 100k args).
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode.apply(
      null,
      Array.from(bytes.subarray(i, i + chunk)),
    );
  }
  return btoa(binary);
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} o`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} Ko`;
  return `${(n / (1024 * 1024)).toFixed(1)} Mo`;
}

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
  const [mode, setMode] = useState<"command" | "file">("command");
  const [commandInput, setCommandInput] = useState<string>(
    'python3 -c "import socket; print(socket.gethostname())"',
  );
  const [fileInterpreter, setFileInterpreter] = useState<string>("python3");
  const [fileName, setFileName] = useState<string>("");
  const [fileArgs, setFileArgs] = useState<string>("");
  const [networkEnabled, setNetworkEnabled] = useState(false);
  const [timeoutSecs, setTimeoutSecs] = useState(60);
  const [workspaceFiles, setWorkspaceFiles] = useState<File[]>([]);
  const [isLaunching, setIsLaunching] = useState(false);
  const [result, setResult] = useState<Task | null>(null);
  const [livePartial, setLivePartial] = useState<Task | null>(null);
  const [error, setError] = useState<string | null>(null);

  const unlistenRef = useRef<UnlistenFn | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const workspaceBytes = useMemo(
    () => workspaceFiles.reduce((sum, f) => sum + f.size, 0),
    [workspaceFiles],
  );
  const workspaceTooBig = workspaceBytes > WORKSPACE_MAX_BYTES;

  // Auto-select first target when the list changes
  useEffect(() => {
    if (!selectedIp && targets.length > 0) {
      setSelectedIp(targets[0].ip);
    } else if (selectedIp && !targets.find((t) => t.ip === selectedIp)) {
      // selected peer disappeared
      setSelectedIp(targets[0]?.ip ?? "");
    }
  }, [targets, selectedIp]);

  // Stop listening on unmount
  useEffect(() => {
    return () => {
      unlistenRef.current?.();
      unlistenRef.current = null;
    };
  }, []);

  const parsedArgs = useMemo(() => {
    if (mode === "command") return parseCommand(commandInput);
    // mode === "file"
    if (!fileName) return [];
    const tail = fileArgs.trim() ? parseCommand(fileArgs) : [];
    return [fileInterpreter, fileName, ...tail];
  }, [mode, commandInput, fileInterpreter, fileName, fileArgs]);

  const stopPolling = () => {
    unlistenRef.current?.();
    unlistenRef.current = null;
  };

  const handleAddFiles = (files: FileList | null) => {
    if (!files || files.length === 0) return;
    const newOnes = Array.from(files);
    // Drop duplicates by name (keep latest)
    const merged = [
      ...workspaceFiles.filter(
        (f) => !newOnes.some((nf) => nf.name === f.name),
      ),
      ...newOnes,
    ];
    setWorkspaceFiles(merged);
    // Reset the input so the same file can be re-picked after removal
    if (fileInputRef.current) fileInputRef.current.value = "";
    // Auto-fill the file selector with the first newly uploaded file if
    // none is selected yet — common case is "I'm in file mode, I upload
    // train.py, I want to run train.py".
    if (!fileName && newOnes.length > 0) {
      setFileName(newOnes[0].name);
    }
    // If the user just uploaded a file but is still in command mode,
    // suggest the file mode.
    if (workspaceFiles.length === 0 && newOnes.length > 0 && mode === "command") {
      setMode("file");
      if (!fileName) setFileName(newOnes[0].name);
    }
  };

  const handleRemoveFile = (name: string) => {
    setWorkspaceFiles((prev) => prev.filter((f) => f.name !== name));
    // If the removed file was the selected one, clear or fallback
    if (fileName === name) {
      const remaining = workspaceFiles.filter((f) => f.name !== name);
      setFileName(remaining[0]?.name ?? "");
    }
  };

  const handleLaunch = async () => {
    if (!selectedIp) {
      setError("Aucun pair sélectionné.");
      return;
    }
    if (parsedArgs.length === 0) {
      setError("La commande est vide.");
      return;
    }
    if (workspaceTooBig) {
      setError(
        `Le workspace dépasse la limite de ${formatBytes(WORKSPACE_MAX_BYTES)}. ` +
          `Total actuel : ${formatBytes(workspaceBytes)}.`,
      );
      return;
    }
    setError(null);
    setIsLaunching(true);
    setResult(null);
    setLivePartial(null);

    // Pre-allocate the outgoing task id so we can poll for live output
    // while the dispatch is still in flight.
    const localId =
      typeof crypto !== "undefined" && "randomUUID" in crypto
        ? crypto.randomUUID()
        : `task-${Date.now()}-${Math.random().toString(16).slice(2)}`;

    // Read + base64 all selected workspace files. Done up-front (not lazily)
    // so the dispatch invoke gets the full payload in one go.
    let workspace: WorkspaceFile[] | undefined;
    if (workspaceFiles.length > 0) {
      try {
        workspace = await Promise.all(
          workspaceFiles.map(async (f) => ({
            path: f.name,
            content_b64: await fileToBase64(f),
          })),
        );
      } catch (e) {
        setError(`Échec de lecture des fichiers : ${String(e)}`);
        setIsLaunching(false);
        return;
      }
    }

    // Subscribe to the live "outgoing-tasks-changed" event so we see
    // progress / output updates the moment the backend pushes them, instead
    // of polling every 500 ms.
    try {
      unlistenRef.current = await listen<Task[]>(
        "outgoing-tasks-changed",
        (e) => {
          const t = e.payload.find((task) => task.id === localId);
          if (t) setLivePartial(t);
        },
      );
    } catch {
      // listen() failure is non-fatal — final result is still delivered
      // by the dispatchTask() promise below.
    }

    try {
      const task = await dispatchTask(selectedIp, parsedArgs, {
        timeoutSecs,
        network: networkEnabled,
        localId,
        workspace,
      });
      setResult(task);
      onDispatched?.();
    } catch (e) {
      setError(String(e));
    } finally {
      stopPolling();
      setIsLaunching(false);
      // Keep livePartial visible if we have a final result; clear otherwise
      // to free state.
      setLivePartial(null);
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

  // Prefer the final result once available; otherwise show the live partial
  // task being polled while the dispatch is in flight.
  const displayedTask = result ?? livePartial;
  const statusInfo = displayedTask
    ? STATUS_LABELS[displayedTask.status] ?? {
        label: displayedTask.status,
        className: "",
      }
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

        <div className="task-dispatcher__field">
          <span className="task-dispatcher__label">Quoi exécuter</span>
          <div className="task-dispatcher__mode-tabs">
            <button
              type="button"
              className={`task-dispatcher__mode-tab${
                mode === "command" ? " task-dispatcher__mode-tab--active" : ""
              }`}
              onClick={() => setMode("command")}
              disabled={isLaunching}
            >
              Une commande
            </button>
            <button
              type="button"
              className={`task-dispatcher__mode-tab${
                mode === "file" ? " task-dispatcher__mode-tab--active" : ""
              }`}
              onClick={() => setMode("file")}
              disabled={isLaunching}
            >
              Un fichier uploadé
            </button>
          </div>

          {mode === "command" ? (
            <>
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
            </>
          ) : (
            <div className="task-dispatcher__file-mode">
              <div className="task-dispatcher__file-mode-row">
                <select
                  value={fileInterpreter}
                  onChange={(e) => setFileInterpreter(e.target.value)}
                  disabled={isLaunching}
                  className="task-dispatcher__input task-dispatcher__input--mono"
                  style={{ width: 120 }}
                >
                  <option value="python3">python3</option>
                  <option value="bash">bash</option>
                  <option value="sh">sh</option>
                </select>
                <select
                  value={fileName}
                  onChange={(e) => setFileName(e.target.value)}
                  disabled={isLaunching || workspaceFiles.length === 0}
                  className="task-dispatcher__input task-dispatcher__input--mono"
                  style={{ flex: 1 }}
                >
                  {workspaceFiles.length === 0 ? (
                    <option value="">— uploadez un fichier d'abord —</option>
                  ) : (
                    <>
                      {!fileName && <option value="">— choisir —</option>}
                      {workspaceFiles.map((f) => (
                        <option key={f.name} value={f.name}>
                          {f.name}
                        </option>
                      ))}
                    </>
                  )}
                </select>
              </div>
              <input
                type="text"
                value={fileArgs}
                onChange={(e) => setFileArgs(e.target.value)}
                disabled={isLaunching}
                placeholder="arguments optionnels (ex: --epochs 10)"
                spellCheck={false}
                autoComplete="off"
                className="task-dispatcher__input task-dispatcher__input--mono"
              />
              {parsedArgs.length > 0 ? (
                <small className="task-dispatcher__parsed">
                  argv : <code>{JSON.stringify(parsedArgs)}</code>
                </small>
              ) : workspaceFiles.length === 0 ? (
                <small className="task-dispatcher__parsed">
                  Uploadez un fichier dans la section ci-dessous.
                </small>
              ) : null}
            </div>
          )}
        </div>

        {mode === "file" && (
        <div className="task-dispatcher__workspace">
          <div className="task-dispatcher__workspace-header">
            <span className="task-dispatcher__label">
              Fichiers à uploader
            </span>
            <button
              type="button"
              className="btn btn--secondary btn--small"
              onClick={() => fileInputRef.current?.click()}
              disabled={isLaunching}
            >
              Ajouter…
            </button>
            <input
              ref={fileInputRef}
              type="file"
              multiple
              hidden
              onChange={(e) => handleAddFiles(e.target.files)}
            />
          </div>
          <p className="task-dispatcher__help">
            Ces fichiers seront copiés dans le répertoire de travail de la
            commande sur le pair (par défaut <code>/workspace</code>).
            Référez-y dans la commande par leur nom (ex.{" "}
            <code>python3 train.py</code>). Limite totale :{" "}
            {formatBytes(WORKSPACE_MAX_BYTES)}.
          </p>
          {workspaceFiles.length > 0 && (
            <ul className="task-dispatcher__files">
              {workspaceFiles.map((f) => (
                <li key={f.name}>
                  <code>{f.name}</code>
                  <span className="task-dispatcher__file-size">
                    {formatBytes(f.size)}
                  </span>
                  <button
                    type="button"
                    className="task-dispatcher__file-remove"
                    onClick={() => handleRemoveFile(f.name)}
                    disabled={isLaunching}
                    title="Retirer ce fichier"
                  >
                    ×
                  </button>
                </li>
              ))}
              <li className="task-dispatcher__files-total">
                Total : {formatBytes(workspaceBytes)}
                {workspaceTooBig && (
                  <span style={{ color: "var(--color-danger)", marginLeft: 8 }}>
                    (dépasse la limite)
                  </span>
                )}
              </li>
            </ul>
          )}
        </div>
        )}

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
            joindre un autre service du réseau local, ou faire un{" "}
            <strong>entraînement DDP</strong> (les processus parallèles
            doivent se synchroniser via le réseau).
          </p>
        </div>

        <div className="task-dispatcher__actions">
          <button
            type="button"
            onClick={handleLaunch}
            disabled={
              isLaunching ||
              !selectedIp ||
              parsedArgs.length === 0 ||
              workspaceTooBig
            }
            className="btn btn--primary"
          >
            {isLaunching ? "Exécution..." : "Lancer"}
          </button>
        </div>
      </div>

      {error ? <div className="alert alert--error">{error}</div> : null}

      {displayedTask ? (
        <div className="task-dispatcher__result">
          <div className="task-dispatcher__result-header">
            <span className={`badge ${statusInfo!.className}`}>
              {statusInfo!.label}
            </span>
            <span style={{ marginLeft: 12 }}>
              cible : <strong>{displayedTask.target_machine}</strong>
              {result ? (
                <>
                  {" · "}exit code : <strong>{result.exit_code ?? "—"}</strong>
                </>
              ) : (
                <> · sortie en direct…</>
              )}
            </span>
          </div>
          {displayedTask.output ? (
            <details open>
              <summary>stdout ({displayedTask.output.length} car.)</summary>
              <pre className="task-dispatcher__pre">{displayedTask.output}</pre>
            </details>
          ) : null}
          {displayedTask.error_output ? (
            <details open={!displayedTask.output}>
              <summary>
                stderr ({displayedTask.error_output.length} car.)
              </summary>
              <pre className="task-dispatcher__pre task-dispatcher__pre--err">
                {displayedTask.error_output}
              </pre>
            </details>
          ) : null}
          {!displayedTask.output && !displayedTask.error_output ? (
            <p style={{ opacity: 0.6, fontStyle: "italic" }}>
              {result ? "(aucune sortie)" : "En attente de la première ligne de sortie…"}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
