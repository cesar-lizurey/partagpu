import { useEffect, useState, useCallback } from "react";
import { PeerTable } from "../components/PeerTable";
import { TaskList } from "../components/TaskList";
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

  const availablePeers = peers.filter((p) => p.sharing_enabled);

  return (
    <div className="page">
      <h2>Mon utilisation</h2>
      <p className="page__subtitle">
        Ce que j'utilise sur les autres machines du réseau
      </p>

      {error && <div className="alert alert--error">{error}</div>}

      <section className="section">
        <h3>Machines disponibles</h3>
        <PeerTable
          peers={availablePeers}
          emptyMessage="Aucune machine ne partage ses ressources actuellement."
        />
      </section>

      <section className="section">
        <h3>Toutes les machines détectées</h3>
        <PeerTable peers={peers} />
      </section>

      <section className="section">
        <h3>Mes tâches en cours</h3>
        <TaskList tasks={tasks} direction="outgoing" />
      </section>
    </div>
  );
}
