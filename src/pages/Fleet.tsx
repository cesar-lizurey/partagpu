import { useEffect, useState, useCallback, useMemo } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getPeers, getOutgoingTasks } from "../lib/api";
import type { Peer, Task, TaskStatus } from "../lib/api";

const ACTIVE_STATUSES: ReadonlySet<TaskStatus> = new Set(["Queued", "Running"]);

/** Fleet overview: an aggregate dashboard of every visible peer in the room.
 *  Shows what each peer offers (capacity), what I'm running on it right now,
 *  and a top-line summary of total fleet capacity vs. my current usage.
 *
 *  Limitation: only shows MY tasks per peer (we don't have a remote
 *  endpoint that exposes another peer's task list yet). For a "see what
 *  every classmate is running" view, a `/peer/v1/status` endpoint would
 *  be needed — see TODO.md. */
export function Fleet() {
  const [peers, setPeers] = useState<Peer[]>([]);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [p, t] = await Promise.all([getPeers(), getOutgoingTasks()]);
      setPeers(p);
      setTasks(t);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, 3000);
    let unlisten: UnlistenFn | undefined;
    listen<Task[]>("outgoing-tasks-changed", (e) => {
      setTasks(e.payload);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      clearInterval(interval);
      unlisten?.();
    };
  }, [refresh]);

  // Group active outgoing tasks by their target peer's display name. We match
  // via display_name first (then hostname) since Task.target_machine is a
  // display string, not an IP.
  const tasksByPeer = useMemo(() => {
    const map = new Map<string, Task[]>();
    for (const task of tasks) {
      if (!ACTIVE_STATUSES.has(task.status)) continue;
      const list = map.get(task.target_machine) ?? [];
      list.push(task);
      map.set(task.target_machine, list);
    }
    return map;
  }, [tasks]);

  // Sort: usable peers first (verified AND sharing), then verified-only,
  // then sharing-only, then the rest. Alphabetical within each bucket.
  const sortedPeers = useMemo(() => {
    return [...peers].sort((a, b) => {
      const usableA = a.verified && a.sharing_enabled;
      const usableB = b.verified && b.sharing_enabled;
      if (usableA !== usableB) return usableA ? -1 : 1;
      if (a.verified !== b.verified) return a.verified ? -1 : 1;
      if (a.sharing_enabled !== b.sharing_enabled)
        return a.sharing_enabled ? -1 : 1;
      return (a.display_name || a.hostname).localeCompare(
        b.display_name || b.hostname,
      );
    });
  }, [peers]);

  // Aggregates for the banner.
  const usable = peers.filter((p) => p.verified && p.sharing_enabled);
  const totalGpus = usable.reduce((s, p) => s + (p.gpu_count ?? 0), 0);
  const myActiveTasks = tasks.filter((t) => ACTIVE_STATUSES.has(t.status));
  const myCpuSum = myActiveTasks.reduce((s, t) => s + t.cpu_usage, 0);
  const myRamSum = myActiveTasks.reduce((s, t) => s + t.ram_usage_mb, 0);
  const myGpuSum = myActiveTasks.reduce((s, t) => s + t.gpu_usage, 0);

  return (
    <div className="page">
      <h2>Vue parc</h2>
      <p className="page__subtitle">
        État global de toutes les machines de la salle
      </p>

      {error && <div className="alert alert--error">{error}</div>}

      <section className="fleet__summary">
        <SummaryStat label="Pairs visibles" value={peers.length} />
        <SummaryStat
          label="Pairs utilisables"
          value={usable.length}
          hint="Auth OK + Partage Actif"
        />
        <SummaryStat
          label="GPU dans la salle"
          value={totalGpus}
          hint="Somme sur les pairs utilisables"
        />
        <SummaryStat
          label="Mes tâches actives"
          value={myActiveTasks.length}
        />
        <SummaryStat
          label="CPU total (mes tâches)"
          value={`${myCpuSum.toFixed(0)} %`}
        />
        <SummaryStat
          label="RAM totale"
          value={`${myRamSum} Mo`}
        />
        <SummaryStat
          label="GPU total"
          value={`${myGpuSum.toFixed(0)} %`}
        />
      </section>

      {peers.length === 0 ? (
        <p className="empty-state">
          Aucun pair détecté pour l'instant. Vérifiez que d'autres machines
          tournent PartaGPU sur le même sous-réseau et qu'elles ont rejoint
          la même salle.
        </p>
      ) : (
        <div className="fleet__grid">
          {sortedPeers.map((peer) => {
            const peerTasks =
              tasksByPeer.get(peer.display_name) ??
              tasksByPeer.get(peer.hostname) ??
              [];
            return (
              <PeerCard key={peer.id} peer={peer} myTasks={peerTasks} />
            );
          })}
        </div>
      )}

      <p className="section__hint">
        Cette vue montre ce que <strong>vous</strong> exécutez sur chaque
        pair. Pour savoir ce que d'autres classmates dispatchent sur eux,
        une route <code>/peer/v1/status</code> agrégée serait nécessaire —
        cf. TODO.md.
      </p>
    </div>
  );
}

function SummaryStat({
  label,
  value,
  hint,
}: {
  label: string;
  value: number | string;
  hint?: string;
}) {
  return (
    <div className="fleet__stat">
      <div className="fleet__stat-value">{value}</div>
      <div className="fleet__stat-label">{label}</div>
      {hint && <div className="fleet__stat-hint">{hint}</div>}
    </div>
  );
}

function PeerCard({ peer, myTasks }: { peer: Peer; myTasks: Task[] }) {
  const usable = peer.verified && peer.sharing_enabled;
  const ramLimit =
    peer.ram_limit > 0
      ? `${Math.round(peer.ram_limit)} Mo`
      : "illimitée";

  return (
    <div className={`peer-card ${usable ? "" : "peer-card--unusable"}`}>
      <div className="peer-card__header">
        <h3 className="peer-card__name">
          {peer.display_name || peer.hostname}
        </h3>
        <code className="peer-card__ip">{peer.ip}</code>
      </div>

      <div className="peer-card__badges">
        <span
          className={`badge ${peer.verified ? "badge--ok" : "badge--warn"}`}
        >
          Auth : {peer.verified ? "OK" : "?"}
        </span>
        <span
          className={`badge ${peer.sharing_enabled ? "badge--ok" : "badge--off"}`}
        >
          Partage : {peer.sharing_enabled ? "Actif" : "—"}
        </span>
        {peer.hostname_conflict && (
          <span className="badge badge--alert">conflit hostname</span>
        )}
      </div>

      <dl className="peer-card__caps">
        <div>
          <dt>GPU</dt>
          <dd>
            {peer.gpu_count ?? 0} ×<br />
            <small>limite {Math.round(peer.gpu_limit)} %</small>
          </dd>
        </div>
        <div>
          <dt>CPU</dt>
          <dd>{Math.round(peer.cpu_limit)} %</dd>
        </div>
        <div>
          <dt>RAM</dt>
          <dd>{ramLimit}</dd>
        </div>
      </dl>

      <div className="peer-card__tasks">
        <strong>Mes tâches ici : {myTasks.length}</strong>
        {myTasks.length === 0 ? (
          <span className="peer-card__idle">
            {usable ? "Disponible" : "Indisponible"}
          </span>
        ) : (
          <ul className="peer-card__task-list">
            {myTasks.map((t) => (
              <li key={t.id}>
                <span className="peer-card__task-cmd">{truncate(t.command, 50)}</span>
                <span className="peer-card__task-metrics">
                  {t.cpu_usage.toFixed(0)} % CPU · {t.ram_usage_mb} Mo ·{" "}
                  {t.gpu_usage.toFixed(0)} % GPU
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function truncate(s: string, max: number): string {
  return s.length > max ? s.slice(0, max) + "…" : s;
}
