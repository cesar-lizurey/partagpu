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
import { useT } from "../lib/i18n";
import type { MessageKey } from "../lib/messages";

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

const STATUS_INFO: Record<string, { key: MessageKey; className: string }> = {
  Queued: { key: "task.status_queued", className: "badge--queued" },
  Running: { key: "task.status_running", className: "badge--running" },
  Completed: { key: "task.status_completed", className: "badge--completed" },
  Failed: { key: "task.status_failed", className: "badge--failed" },
  Cancelled: { key: "task.status_cancelled", className: "badge--disabled" },
};

export function DDPDispatcher({ peers }: DDPDispatcherProps) {
  const t = useT();
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
      setError(t("ddp.err_no_script"));
      return;
    }
    if (totalSelectedGpus < 1) {
      setError(t("ddp.err_no_gpu"));
      return;
    }
    if (workspaceTooBig) {
      setError(
        t("ddp.err_workspace_too_big", {
          size: formatBytes(workspaceBytes),
          limit: formatBytes(WORKSPACE_MAX_BYTES),
        }),
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
      setError(t("ddp.err_read_files", { error: String(e) }));
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
    // for the NCCL/Gloo rendezvous to converge. The first rank to fail
    // triggers a cancel-all so siblings stuck in the rendezvous don't
    // hang for the full timeout.
    const settledIds = new Set<string>();
    let abortTriggered = false;
    const triggerAbort = (sourceRank: number, reason: string) => {
      if (abortTriggered) return;
      abortTriggered = true;
      const target = newAssignments.find((a) => a.rank === sourceRank);
      setError(
        target
          ? t("ddp.abort_with_target", {
              rank: sourceRank,
              peer: target.peer.display_name || target.peer.hostname,
              reason,
            })
          : t("ddp.abort_no_target", { rank: sourceRank, reason }),
      );
      // Cancel every sibling that hasn't already settled.
      for (const a of newAssignments) {
        if (a.rank === sourceRank) continue;
        if (settledIds.has(a.localId)) continue;
        cancelOutgoingTask(a.localId).catch(() => undefined);
      }
    };

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
        const result = await dispatchTask(a.peer.ip, cmd, {
          timeoutSecs,
          network: true,
          user: `${userLabel} (rank ${a.rank}/${worldSize}, dev ${a.deviceIndex})`,
          localId: a.localId,
          workspace,
        });
        settledIds.add(a.localId);
        // A non-zero exit code means this rank crashed — others are likely
        // hung in NCCL rendezvous waiting for it.
        if (result.status === "Failed" && !abortTriggered) {
          triggerAbort(a.rank, `exit ${result.exit_code ?? "?"}`);
        }
        return result;
      } catch (e) {
        settledIds.add(a.localId);
        if (!abortTriggered) {
          triggerAbort(a.rank, String(e));
        }
        throw new Error(`Rank ${a.rank} : ${String(e)}`);
      }
    });

    try {
      const settled = await Promise.allSettled(dispatchPromises);
      const failures = settled
        .map((r, i) => (r.status === "rejected" ? `R${i}: ${r.reason}` : null))
        .filter(Boolean);
      if (failures.length > 0 && !abortTriggered) {
        // Rare path: failures arrived after abort window. Surface them anyway.
        setError(t("ddp.errors", { list: failures.join(" ; ") }));
      }
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
        <p className="empty-state">{t("ddp.no_targets")}</p>
      </div>
    );
  }

  return (
    <div className="ddp-dispatcher">
      <p className="ddp-dispatcher__intro">
        {t("ddp.intro_p1")}
        <code>MASTER_ADDR</code>
        {t("ddp.intro_p2")}
        <code>MASTER_PORT</code>
        {t("ddp.intro_p2")}
        <code>RANK</code>
        {t("ddp.intro_p2")}
        <code>WORLD_SIZE</code>
        {t("ddp.intro_p2")}
        <code>CUDA_VISIBLE_DEVICES</code>
        {t("ddp.intro_p2")}
        <code>BACKEND</code>
        {t("ddp.intro_p3")}
      </p>

      <div className="ddp-dispatcher__form">
        <fieldset className="ddp-dispatcher__peers">
          <legend>{t("ddp.targets_legend", { n: totalSelectedGpus })}</legend>
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
                  title={t("ddp.peer_max_title", { n: max })}
                />
                <span className="ddp-dispatcher__peer-max">/ {max}</span>
              </div>
            );
          })}
        </fieldset>

        <div className="ddp-dispatcher__row">
          <label className="ddp-dispatcher__field">
            <span>{t("ddp.backend_label")}</span>
            <select
              value={backend}
              onChange={(e) => setBackend(e.target.value as "nccl" | "gloo")}
              disabled={isLaunching}
            >
              <option value="nccl">{t("ddp.backend_nccl")}</option>
              <option value="gloo">{t("ddp.backend_gloo")}</option>
            </select>
          </label>
          <label className="ddp-dispatcher__field ddp-dispatcher__field--narrow">
            <span>{t("ddp.master_port_label")}</span>
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
            <span>{t("ddp.timeout_label")}</span>
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
          <span>{t("ddp.script_label")}</span>
          <div className="ddp-dispatcher__file-row">
            <button
              type="button"
              className="btn btn--secondary btn--small"
              onClick={() => scriptInputRef.current?.click()}
              disabled={isLaunching}
            >
              {t("ddp.script_pick")}
            </button>
            <input
              ref={scriptInputRef}
              type="file"
              hidden
              accept=".py,.sh"
              onChange={(e) => handleScriptPick(e.target.files)}
            />
            <code>{scriptFile ? scriptFile.name : t("ddp.script_none")}</code>
            {scriptFile && (
              <span className="ddp-dispatcher__file-size">
                {formatBytes(scriptFile.size)}
              </span>
            )}
          </div>
        </div>

        <div className="ddp-dispatcher__field">
          <span>{t("ddp.script_args_label")}</span>
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
          <span>{t("ddp.extras_label")}</span>
          <div className="ddp-dispatcher__file-row">
            <button
              type="button"
              className="btn btn--secondary btn--small"
              onClick={() => extraInputRef.current?.click()}
              disabled={isLaunching}
            >
              {t("ddp.extras_add")}
            </button>
            <input
              ref={extraInputRef}
              type="file"
              multiple
              hidden
              onChange={(e) => handleAddExtras(e.target.files)}
            />
            <small>
              {t("ddp.extras_limit_hint", { limit: formatBytes(WORKSPACE_MAX_BYTES) })}
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
              ? t("ddp.btn_launching", { n: totalSelectedGpus })
              : t("ddp.btn_launch", { n: totalSelectedGpus })}
          </button>
          {isLaunching && assignments.length > 0 && (
            <button
              type="button"
              className="btn btn--secondary"
              onClick={() => void handleCancelAll()}
            >
              {t("ddp.btn_cancel_all")}
            </button>
          )}
        </div>

        {error && <div className="alert alert--error">{error}</div>}
      </div>

      {assignments.length > 0 && (
        <div className="ddp-dispatcher__ranks">
          <h4>{t("ddp.ranks_title")}</h4>
          <table className="ddp-dispatcher__rank-table">
            <thead>
              <tr>
                <th>{t("ddp.col_rank")}</th>
                <th>{t("ddp.col_peer")}</th>
                <th>{t("ddp.col_gpu")}</th>
                <th>{t("ddp.col_state")}</th>
                <th>{t("ddp.col_progress")}</th>
              </tr>
            </thead>
            <tbody>
              {assignments.map((a) => {
                const task = taskStates[a.localId];
                const status = task?.status ?? "Queued";
                const info = STATUS_INFO[status];
                const label = info ? t(info.key) : status;
                const className = info?.className ?? "";
                return (
                  <tr key={a.localId}>
                    <td>{a.rank}</td>
                    <td>
                      {a.peer.display_name || a.peer.hostname}{" "}
                      <small>({a.peer.ip})</small>
                    </td>
                    <td>{t("ddp.dev_label", { n: a.deviceIndex })}</td>
                    <td>
                      <span className={`badge ${className}`}>{label}</span>
                    </td>
                    <td>
                      <div className="ddp-dispatcher__progress">
                        <div
                          className="ddp-dispatcher__progress-fill"
                          style={{ width: `${task?.progress ?? 0}%` }}
                        />
                        <span>{Math.round(task?.progress ?? 0)}%</span>
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
