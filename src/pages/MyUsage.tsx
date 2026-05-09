import { useEffect, useState, useCallback } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { PeerTable } from "../components/PeerTable";
import { TaskList } from "../components/TaskList";
import { TaskDispatcher } from "../components/TaskDispatcher";
import { DDPDispatcher } from "../components/DDPDispatcher";
import { getPeers, getOutgoingTasks } from "../lib/api";
import { useTaskCompletionNotifications } from "../lib/notifications";
import type { Peer, Task } from "../lib/api";
import { useT } from "../lib/i18n";

export function MyUsage() {
  const t = useT();
  const [peers, setPeers] = useState<Peer[]>([]);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [error, setError] = useState<string | null>(null);

  // Desktop toast whenever an outgoing task hits Completed/Failed/Cancelled.
  // Lets the user step away during a long DDP run and still get notified.
  useTaskCompletionNotifications(tasks);

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
    // Peers come from mDNS — keep polling at 3s. Outgoing tasks are pushed
    // by the backend via the "outgoing-tasks-changed" Tauri event below, so
    // we don't need a fast poll for live progress / output.
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

  // Sort : usable peers (verified AND sharing) at the very top, since
  // they're the only ones we can actually dispatch to. Then verified-but-
  // not-sharing, then sharing-but-unverified (still unusable), then
  // neither. Alphabetical within each bucket.
  const sortedPeers = [...peers].sort((a, b) => {
    const usableA = a.verified && a.sharing_enabled;
    const usableB = b.verified && b.sharing_enabled;
    if (usableA !== usableB) return usableA ? -1 : 1;
    if (a.verified !== b.verified) return a.verified ? -1 : 1;
    if (a.sharing_enabled !== b.sharing_enabled) {
      return a.sharing_enabled ? -1 : 1;
    }
    return (a.display_name || a.hostname).localeCompare(
      b.display_name || b.hostname,
    );
  });

  return (
    <div className="page">
      <h2>{t("myusage.title")}</h2>
      <p className="page__subtitle">{t("myusage.subtitle")}</p>

      {error && <div className="alert alert--error">{error}</div>}

      <section className="section">
        <h3>{t("myusage.peers_section")}</h3>
        <p className="section__hint">
          {t("myusage.peers_hint_p1")}
          <strong>{t("myusage.peers_hint_authok")}</strong>
          {t("myusage.peers_hint_p2")}
          <strong>{t("myusage.peers_hint_active")}</strong>
          {t("myusage.peers_hint_p3")}
          <strong>{t("myusage.peers_hint_qmark")}</strong>
          {t("myusage.peers_hint_p4")}
        </p>
        <PeerTable peers={sortedPeers} />
      </section>

      <section className="section">
        <h3>{t("myusage.command_section")}</h3>
        <TaskDispatcher peers={peers} onDispatched={refresh} />
      </section>

      <section className="section">
        <h3>{t("myusage.ddp_section")}</h3>
        <DDPDispatcher peers={peers} />
      </section>

      <section className="section">
        <h3>{t("myusage.tasks_section")}</h3>
        <TaskList tasks={tasks} direction="outgoing" onCancelled={refresh} />
      </section>
    </div>
  );
}
