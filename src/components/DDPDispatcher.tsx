import { useEffect, useMemo, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  cancelOutgoingTask,
  dispatchTask,
  getMachineInfo,
  type MachineInfo,
  type Peer,
  type Task,
  type WorkspaceFile,
} from "../lib/api";

const WORKSPACE_MAX_BYTES = 16 * 1024 * 1024;
const DEFAULT_MASTER_PORT = 29500;

async function fileToBase64(file: File): Promise<string> {
  const buf = await file.arrayBuffer();
  const bytes = new Uint8Array(buf);
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

function parseExtraArgs(input: string): string[] {
  // Reuse the same shell-like parse as TaskDispatcher (single + double quotes
  // and backslash escapes). Kept simple — env var / glob expansion stays
  // off-limits to avoid surprising the user.
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

interface DDPDispatcherProps {
  /** All known peers; this component filters down to verified+sharing
   *  with at least one GPU advertised. */
  peers: Peer[];
}

interface RankAssignment {
  /** rank in the global world. */
  rank: number;
  /** zero-based index among workers running on the same host. */
  localRank: number;
  /** world size (total ranks). */
  worldSize: number;
  /** target peer (with display info). */
  peer: Peer;
  /** which CUDA device on that peer this rank gets. */
  deviceIndex: number;
  /** localId used by the dispatcher (for live event matching + cancel). */
  localId: string;
}

const STATUS_LABELS: Record<string, { label: string; className: string }> = {
  Queued: { label: "En attente", className: "badge--queued" },
  Running: { label: "En cours", className: "badge--running" },
  Completed: { label: "Terminée", className: "badge--completed" },
  Failed: { label: "Échouée", className: "badge--failed" },
  Cancelled: { label: "Annulée", className: "badge--disabled" },
};

export function DDPDispatcher({ peers }: DDPDispatcherProps) {
  const targets = useMemo(
    () =>
      peers.filter(
        (p) => p.verified && p.sharing_enabled && (p.gpu_count ?? 0) > 0,
      ),
    [peers],
  );

  // Per peer: how many of its GPUs to use (default 1 for each selected peer).
  // 0 means "not selected".
  const [selectedGpus, setSelectedGpus] = useState<Record<string, number>>({});
  const [scriptFile, setScriptFile] = useState<File | null>(null);
  const [extraFiles, setExtraFiles] = useState<File[]>([]);
  const [extraArgs, setExtraArgs] = useState<string>("");
  const [backend, setBackend] = useState<"nccl" | "gloo">("nccl");
  const [masterPort, setMasterPort] = useState<number>(DEFAULT_MASTER_PORT);
  const [timeoutSecs, setTimeoutSecs] = useState<number>(1800);
  const [machine, setMachine] = useState<MachineInfo | null>(null);

  const [isLaunching, setIsLaunching] = useState(false);
  const [assignments, setAssignments] = useState<RankAssignment[]>([]);
  const [taskStates, setTaskStates] = useState<Record<string, Task>>({});
  const [error, setError] = useState<string | null>(null);

  const scriptInputRef = useRef<HTMLInputElement | null>(null);
  const extraInputRef = useRef<HTMLInputElement | null>(null);
  const unlistenRef = useRef<UnlistenFn | null>(null);

  useEffect(() => {
    getMachineInfo().then(setMachine).catch(() => undefined);
  }, []);

  useEffect(() => {
    return () => {
      unlistenRef.current?.();
      unlistenRef.current = null;
    };
  }, []);

  const totalSelectedGpus = useMemo(
    () => Object.values(selectedGpus).reduce((a, b) => a + b, 0),
    [selectedGpus],
  );

  const workspaceBytes = useMemo(() => {
    const all = scriptFile ? [scriptFile, ...extraFiles] : extraFiles;
    return all.reduce((sum, f) => sum + f.size, 0);
  }, [scriptFile, extraFiles]);
  const workspaceTooBig = workspaceBytes > WORKSPACE_MAX_BYTES;

  const setGpuCount = (peerId: string, count: number, max: number) => {
    const c = Math.max(0, Math.min(max, Math.floor(count)));
    setSelectedGpus((prev) => ({ ...prev, [peerId]: c }));
  };

  const togglePeer = (peer: Peer) => {
    const current = selectedGpus[peer.id] ?? 0;
    if (current > 0) {
      setSelectedGpus((prev) => ({ ...prev, [peer.id]: 0 }));
    } else {
      setSelectedGpus((prev) => ({ ...prev, [peer.id]: peer.gpu_count ?? 1 }));
    }
  };

  const handleScriptPick = (files: FileList | null) => {
    if (!files || files.length === 0) return;
    setScriptFile(files[0]);
    if (scriptInputRef.current) scriptInputRef.current.value = "";
  };

  const handleAddExtras = (files: FileList | null) => {
    if (!files || files.length === 0) return;
    const newOnes = Array.from(files);
    setExtraFiles((prev) => [
      ...prev.filter((f) => !newOnes.some((nf) => nf.name === f.name)),
      ...newOnes,
    ]);
    if (extraInputRef.current) extraInputRef.current.value = "";
  };

  const handleRemoveExtra = (name: string) => {
    setExtraFiles((prev) => prev.filter((f) => f.name !== name));
  };

  const handleLaunch = async () => {
    setError(null);
    if (!scriptFile) {
      setError("Sélectionnez un script Python à exécuter.");
      return;
    }
    if (totalSelectedGpus < 1) {
      setError("Sélectionnez au moins un GPU sur un pair.");
      return;
    }
    if (workspaceTooBig) {
      setError(
        `Workspace trop volumineux (${formatBytes(workspaceBytes)} / ${formatBytes(WORKSPACE_MAX_BYTES)}).`,
      );
      return;
    }

    // Build the per-rank assignment list deterministically: walk peers in
    // peer-id order, then GPU index 0..N-1 per peer.
    const sortedPeers = [...targets].sort((a, b) => a.id.localeCompare(b.id));
    const flat: { peer: Peer; deviceIndex: number }[] = [];
    for (const peer of sortedPeers) {
      const want = selectedGpus[peer.id] ?? 0;
      for (let i = 0; i < want; i++) {
        flat.push({ peer, deviceIndex: i });
      }
    }
    const worldSize = flat.length;

    // Master = rank 0's IP. If rank 0 ran on this machine via loopback we'd
    // need to substitute the LAN IP, but currently we never dispatch to
    // ourselves through this UI (peers list excludes self).
    const masterAddr = flat[0].peer.ip;

    // LOCAL_RANK = position among workers on the same host (by IP).
    const seenPerIp: Record<string, number> = {};
    const newAssignments: RankAssignment[] = flat.map((entry, rank) => {
      const localRank = seenPerIp[entry.peer.ip] ?? 0;
      seenPerIp[entry.peer.ip] = localRank + 1;
      return {
        rank,
        localRank,
        worldSize,
        peer: entry.peer,
        deviceIndex: entry.deviceIndex,
        localId:
          typeof crypto !== "undefined" && "randomUUID" in crypto
            ? crypto.randomUUID()
            : `ddp-${Date.now()}-${rank}-${Math.random().toString(16).slice(2)}`,
      };
    });

    // Read all workspace files once. The same payload is shipped to every peer.
    let workspace: WorkspaceFile[];
    try {
      const allFiles = [scriptFile, ...extraFiles];
      workspace = await Promise.all(
        allFiles.map(async (f) => ({
          path: f.name,
          content_b64: await fileToBase64(f),
        })),
      );
    } catch (e) {
      setError(`Échec de lecture des fichiers : ${String(e)}`);
      return;
    }

    setAssignments(newAssignments);
    setTaskStates({});
    setIsLaunching(true);

    // Subscribe to live updates for any of the new local IDs.
    const wantedIds = new Set(newAssignments.map((a) => a.localId));
    try {
      unlistenRef.current = await listen<Task[]>(
        "outgoing-tasks-changed",
        (e) => {
          const updates: Record<string, Task> = {};
          for (const t of e.payload) {
            if (wantedIds.has(t.id)) updates[t.id] = t;
          }
          if (Object.keys(updates).length > 0) {
            setTaskStates((prev) => ({ ...prev, ...updates }));
          }
        },
      );
    } catch {
      /* listener best effort */
    }

    const userLabel =
      machine?.user ?? machine?.display_name ?? "ddp";
    const scriptName = scriptFile.name;
    const tail = parseExtraArgs(extraArgs);

    // Fire all dispatches in parallel; they need to be alive simultaneously
    // for the NCCL/Gloo rendezvous to converge.
    const dispatchPromises = newAssignments.map(async (a) => {
      const envPrefix = [
        "env",
        `MASTER_ADDR=${masterAddr}`,
        `MASTER_PORT=${masterPort}`,
        `RANK=${a.rank}`,
        `WORLD_SIZE=${worldSize}`,
        `LOCAL_RANK=0`,
        `PARTAGPU_LOCAL_RANK=${a.localRank}`,
        `CUDA_VISIBLE_DEVICES=${a.deviceIndex}`,
        `BACKEND=${backend}`,
      ];
      const cmd = [...envPrefix, "python3", scriptName, ...tail];
      try {
        return await dispatchTask(a.peer.ip, cmd, {
          timeoutSecs,
          network: true,
          user: `${userLabel} (rank ${a.rank}/${worldSize}, dev ${a.deviceIndex})`,
          localId: a.localId,
          workspace,
        });
      } catch (e) {
        // Surface the rank that failed. Don't auto-cancel siblings here —
        // the user can do it via the cancel-all button. Keep behavior simple
        // for now.
        throw new Error(`Rank ${a.rank} : ${String(e)}`);
      }
    });

    try {
      await Promise.allSettled(dispatchPromises).then((settled) => {
        const failures = settled
          .map((r, i) => (r.status === "rejected" ? `R${i}: ${r.reason}` : null))
          .filter(Boolean);
        if (failures.length > 0) {
          setError(`Erreurs : ${failures.join(" ; ")}`);
        }
      });
    } finally {
      unlistenRef.current?.();
      unlistenRef.current = null;
      setIsLaunching(false);
    }
  };

  const handleCancelAll = async () => {
    await Promise.all(
      assignments.map((a) =>
        cancelOutgoingTask(a.localId).catch(() => false),
      ),
    );
  };

  if (targets.length === 0) {
    return (
      <div className="ddp-dispatcher">
        <p className="empty-state">
          Aucun pair vérifié n'expose de GPU pour le moment. Activez le partage
          côté camarade et vérifiez que vous êtes dans la même salle.
        </p>
      </div>
    );
  }

  return (
    <div className="ddp-dispatcher">
      <p className="ddp-dispatcher__intro">
        Lance un script Python en mode DDP (un processus par GPU sélectionné,
        rendez-vous NCCL/Gloo sur le LAN). Le script et ses dépendances sont
        envoyés à chaque pair ; les variables d'environnement{" "}
        <code>MASTER_ADDR</code>, <code>MASTER_PORT</code>, <code>RANK</code>,{" "}
        <code>WORLD_SIZE</code>, <code>CUDA_VISIBLE_DEVICES</code>,{" "}
        <code>BACKEND</code> sont positionnées automatiquement.
      </p>

      <div className="ddp-dispatcher__form">
        <fieldset className="ddp-dispatcher__peers">
          <legend>Cibles ({totalSelectedGpus} GPU sélectionnés)</legend>
          {targets.map((peer) => {
            const max = peer.gpu_count ?? 1;
            const selected = selectedGpus[peer.id] ?? 0;
            return (
              <div key={peer.id} className="ddp-dispatcher__peer-row">
                <label className="ddp-dispatcher__peer-check">
                  <input
                    type="checkbox"
                    checked={selected > 0}
                    onChange={() => togglePeer(peer)}
                    disabled={isLaunching}
                  />
                  <span>
                    {peer.display_name || peer.hostname}{" "}
                    <small>({peer.ip})</small>
                  </span>
                </label>
                <input
                  type="number"
                  min={0}
                  max={max}
                  value={selected}
                  onChange={(e) =>
                    setGpuCount(peer.id, Number(e.target.value), max)
                  }
                  disabled={isLaunching || selected === 0}
                  className="ddp-dispatcher__peer-count"
                  title={`max ${max} GPU`}
                />
                <span className="ddp-dispatcher__peer-max">/ {max}</span>
              </div>
            );
          })}
        </fieldset>

        <div className="ddp-dispatcher__row">
          <label className="ddp-dispatcher__field">
            <span>Backend</span>
            <select
              value={backend}
              onChange={(e) => setBackend(e.target.value as "nccl" | "gloo")}
              disabled={isLaunching}
            >
              <option value="nccl">NCCL (GPU)</option>
              <option value="gloo">Gloo (CPU/GPU)</option>
            </select>
          </label>
          <label className="ddp-dispatcher__field ddp-dispatcher__field--narrow">
            <span>Port maître</span>
            <input
              type="number"
              min={29500}
              max={29510}
              value={masterPort}
              onChange={(e) => setMasterPort(Number(e.target.value))}
              disabled={isLaunching}
            />
          </label>
          <label className="ddp-dispatcher__field ddp-dispatcher__field--narrow">
            <span>Timeout (s)</span>
            <input
              type="number"
              min={30}
              max={86400}
              value={timeoutSecs}
              onChange={(e) => setTimeoutSecs(Number(e.target.value))}
              disabled={isLaunching}
            />
          </label>
        </div>

        <div className="ddp-dispatcher__field">
          <span>Script Python</span>
          <div className="ddp-dispatcher__file-row">
            <button
              type="button"
              className="btn btn--secondary btn--small"
              onClick={() => scriptInputRef.current?.click()}
              disabled={isLaunching}
            >
              Choisir…
            </button>
            <input
              ref={scriptInputRef}
              type="file"
              hidden
              accept=".py,.sh"
              onChange={(e) => handleScriptPick(e.target.files)}
            />
            <code>{scriptFile ? scriptFile.name : "— aucun —"}</code>
            {scriptFile && (
              <span className="ddp-dispatcher__file-size">
                {formatBytes(scriptFile.size)}
              </span>
            )}
          </div>
        </div>

        <div className="ddp-dispatcher__field">
          <span>Arguments du script</span>
          <input
            type="text"
            value={extraArgs}
            onChange={(e) => setExtraArgs(e.target.value)}
            placeholder="--epochs 10 --batch-size 32"
            disabled={isLaunching}
            spellCheck={false}
            autoComplete="off"
          />
        </div>

        <div className="ddp-dispatcher__field">
          <span>Fichiers compagnons</span>
          <div className="ddp-dispatcher__file-row">
            <button
              type="button"
              className="btn btn--secondary btn--small"
              onClick={() => extraInputRef.current?.click()}
              disabled={isLaunching}
            >
              Ajouter…
            </button>
            <input
              ref={extraInputRef}
              type="file"
              multiple
              hidden
              onChange={(e) => handleAddExtras(e.target.files)}
            />
            <small>
              Limite totale (script + compagnons) :{" "}
              {formatBytes(WORKSPACE_MAX_BYTES)}.
            </small>
          </div>
          {extraFiles.length > 0 && (
            <ul className="ddp-dispatcher__files">
              {extraFiles.map((f) => (
                <li key={f.name}>
                  <code>{f.name}</code>
                  <span className="ddp-dispatcher__file-size">
                    {formatBytes(f.size)}
                  </span>
                  <button
                    type="button"
                    onClick={() => handleRemoveExtra(f.name)}
                    disabled={isLaunching}
                    className="ddp-dispatcher__file-remove"
                  >
                    ×
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>

        <div className="ddp-dispatcher__actions">
          <button
            type="button"
            className="btn btn--primary"
            onClick={handleLaunch}
            disabled={
              isLaunching ||
              !scriptFile ||
              totalSelectedGpus < 1 ||
              workspaceTooBig
            }
          >
            {isLaunching
              ? `Lancement… (${totalSelectedGpus} ranks)`
              : `Lancer (${totalSelectedGpus} ranks)`}
          </button>
          {isLaunching && assignments.length > 0 && (
            <button
              type="button"
              className="btn btn--secondary"
              onClick={() => void handleCancelAll()}
            >
              Tout annuler
            </button>
          )}
        </div>

        {error && <div className="alert alert--error">{error}</div>}
      </div>

      {assignments.length > 0 && (
        <div className="ddp-dispatcher__ranks">
          <h4>Ranks</h4>
          <table className="ddp-dispatcher__rank-table">
            <thead>
              <tr>
                <th>Rank</th>
                <th>Pair</th>
                <th>GPU</th>
                <th>État</th>
                <th>Progression</th>
              </tr>
            </thead>
            <tbody>
              {assignments.map((a) => {
                const t = taskStates[a.localId];
                const status = t?.status ?? "Queued";
                const info =
                  STATUS_LABELS[status] ?? { label: status, className: "" };
                return (
                  <tr key={a.localId}>
                    <td>{a.rank}</td>
                    <td>
                      {a.peer.display_name || a.peer.hostname}{" "}
                      <small>({a.peer.ip})</small>
                    </td>
                    <td>dev {a.deviceIndex}</td>
                    <td>
                      <span className={`badge ${info.className}`}>
                        {info.label}
                      </span>
                    </td>
                    <td>
                      <div className="ddp-dispatcher__progress">
                        <div
                          className="ddp-dispatcher__progress-fill"
                          style={{ width: `${t?.progress ?? 0}%` }}
                        />
                        <span>{Math.round(t?.progress ?? 0)}%</span>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
