import { useEffect, useState, useCallback } from "react";
import { PeerTable } from "../components/PeerTable";
import { TaskList } from "../components/TaskList";
import { TaskDispatcher } from "../components/TaskDispatcher";
import { getPeers, getOutgoingTasks } from "../lib/api";
import type { Peer, Task } from "../lib/api";

export function MyUsage() {
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
    return () => clearInterval(interval);
  }, [refresh]);

  // Sort : peers that share their resources first (the only ones you can
  // actually use), then non-sharing peers. Within each group, verified
  // before unverified, then alphabetical.
  const sortedPeers = [...peers].sort((a, b) => {
    if (a.sharing_enabled !== b.sharing_enabled) {
      return a.sharing_enabled ? -1 : 1;
    }
    if (a.verified !== b.verified) {
      return a.verified ? -1 : 1;
    }
    return (a.display_name || a.hostname).localeCompare(
      b.display_name || b.hostname,
    );
  });

  return (
    <div className="page">
      <h2>Mon utilisation</h2>
      <p className="page__subtitle">
        Ce que j'utilise sur les autres machines du réseau
      </p>

      {error && <div className="alert alert--error">{error}</div>}

      <section className="section">
        <h3>Machines détectées</h3>
        <p className="section__hint">
          Vous pouvez utiliser les machines avec <strong>Partage : Actif</strong>{" "}
          (triées en premier). Les autres sont visibles mais ne mettent rien à
          disposition pour le moment.
        </p>
        <PeerTable peers={sortedPeers} />
      </section>

      <section className="section">
        <h3>Lancer une commande sur un pair</h3>
        <TaskDispatcher peers={peers} onDispatched={refresh} />
      </section>

      <section className="section">
        <h3>Mes tâches en cours</h3>
        <TaskList tasks={tasks} direction="outgoing" onCancelled={refresh} />
      </section>
    </div>
  );
}
