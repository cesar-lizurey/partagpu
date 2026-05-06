import { useEffect, useState, useCallback } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ManagedVenvPanel } from "../components/ManagedVenvPanel";
import { ResourceGauge } from "../components/ResourceGauge";
import { SharingToggle } from "../components/SharingToggle";
import { TaskList } from "../components/TaskList";
import { UsageBreakdown } from "../components/UsageBreakdown";
import { SecurityLogPanel } from "../components/SecurityLog";
import {
  getResources,
  getSharingConfig,
  enableSharing,
  disableSharing,
  pauseSharing,
  resumeSharing,
  setSharingLimits,
  getIncomingTasks,
  getUserStatus,
  setUserPassword,
  getMaxConcurrentTasks,
  setMaxConcurrentTasks,
} from "../lib/api";
import type { ResourceUsage, SharingConfig, Task, UserStatus } from "../lib/api";

function UserSetup({
  userStatus,
  onDone,
}: {
  userStatus: UserStatus;
  onDone: () => void;
}) {
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (password.length < 4) {
      setError("Le mot de passe doit contenir au moins 4 caractères.");
      return;
    }
    if (password !== confirm) {
      setError("Les mots de passe ne correspondent pas.");
      return;
    }

    setLoading(true);
    try {
      await setUserPassword(password);
      setPassword("");
      setConfirm("");
      onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const statusMessage: Record<string, string> = {
    Missing:
      "L'utilisateur partagpu n'existe pas encore. Il sera créé en activant le partage.",
    NoLogin:
      "L'utilisateur partagpu existe mais n'a pas de shell de connexion. Activez le partage pour le mettre à jour.",
    NoPassword:
      "L'utilisateur partagpu existe mais n'a pas de mot de passe. Définissez-en un pour permettre la connexion depuis l'écran de login.",
  };

  return (
    <div className="user-setup">
      <div className="user-setup__status">
        <span
          className={`user-setup__dot ${userStatus === "Ready" ? "user-setup__dot--ok" : "user-setup__dot--warn"}`}
        />
        <span>
          {userStatus === "Ready"
            ? "Utilisateur partagpu configuré et prêt à l'emploi."
            : statusMessage[userStatus] || "Statut inconnu."}
        </span>
      </div>

      {(userStatus === "NoPassword" || userStatus === "Ready") && (
        <form className="user-setup__form" onSubmit={handleSubmit}>
          <p className="user-setup__hint">
            {userStatus === "Ready"
              ? "Modifier le mot de passe de l'utilisateur partagpu :"
              : "Définir le mot de passe pour se connecter à cette machine :"}
          </p>
          <div className="user-setup__fields">
            <input
              type="password"
              placeholder="Mot de passe"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="new-password"
            />
            <input
              type="password"
              placeholder="Confirmer"
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              autoComplete="new-password"
            />
            <button className="btn btn--primary" type="submit" disabled={loading}>
              {loading ? "..." : userStatus === "Ready" ? "Modifier" : "Définir"}
            </button>
          </div>
          {error && <p className="user-setup__error">{error}</p>}
        </form>
      )}
    </div>
  );
}

export function MySharing() {
  const [resources, setResources] = useState<ResourceUsage | null>(null);
  const [config, setConfig] = useState<SharingConfig | null>(null);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [userStatus, setUserStatus] = useState<UserStatus>("Missing");
  const [maxConcurrent, setMaxConcurrent] = useState<number>(4);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [res, cfg, t, us, mc] = await Promise.all([
        getResources(),
        getSharingConfig(),
        getIncomingTasks(),
        getUserStatus(),
        getMaxConcurrentTasks(),
      ]);
      setResources(res);
      setConfig(cfg);
      setTasks(t);
      setUserStatus(us);
      setMaxConcurrent(mc);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const handleConcurrencyChange = async (n: number) => {
    const clamped = Math.max(1, Math.min(64, Math.floor(n)));
    setMaxConcurrent(clamped);
    try {
      await setMaxConcurrentTasks(clamped);
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    refresh();
    // Resources/sharing-config still polled at 3 s. Incoming tasks are pushed
    // via the "incoming-tasks-changed" Tauri event for instant progress / output
    // updates without a tight polling loop.
    const interval = setInterval(refresh, 3000);
    let unlisten: UnlistenFn | undefined;
    listen<Task[]>("incoming-tasks-changed", (e) => {
      setTasks(e.payload);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      clearInterval(interval);
      unlisten?.();
    };
  }, [refresh]);

  const handleAction = async (action: () => Promise<SharingConfig>) => {
    try {
      const cfg = await action();
      setConfig(cfg);
      setError(null);
      // Refresh user status after enable (user may have been created)
      const us = await getUserStatus();
      setUserStatus(us);
    } catch (e) {
      setError(String(e));
    }
  };

  const handleLimitsChange = async (cpu: number, ram: number, gpu: number) => {
    try {
      const cfg = await setSharingLimits(cpu, ram, gpu);
      setConfig(cfg);
    } catch (e) {
      setError(String(e));
    }
  };

  // Per-resource limit setter used by the gauges. Reads the latest config so
  // changing one limit doesn't reset the others.
  const setLimit = (field: "cpu" | "ram" | "gpu", value: number) => {
    if (!config) return;
    const cpu = field === "cpu" ? value : config.cpu_limit_percent;
    const ram = field === "ram" ? value : config.ram_limit_mb;
    const gpu = field === "gpu" ? value : config.gpu_limit_percent;
    void handleLimitsChange(cpu, ram, gpu);
  };

  return (
    <div className="page">
      <h2>Mon partage</h2>
      <p className="page__subtitle">
        Ce que les autres utilisent sur cette machine
      </p>

      {error && <div className="alert alert--error">{error}</div>}

      {config && (
        <SharingToggle
          status={config.status}
          onEnable={() => handleAction(enableSharing)}
          onDisable={() => handleAction(disableSharing)}
          onPause={() => handleAction(pauseSharing)}
          onResume={() => handleAction(resumeSharing)}
        />
      )}

      {config && config.status !== "Disabled" && (
        <section className="section">
          <h3>Compte partagpu</h3>
          <UserSetup userStatus={userStatus} onDone={refresh} />
        </section>
      )}

      {resources && (
        <section className="section">
          <h3>Ressources de cette machine</h3>
          {config && config.status !== "Disabled" && (
            <p className="section__hint">
              Faites glisser le curseur rouge sur chaque jauge pour ajuster
              la limite que vous partagez aux autres.
            </p>
          )}
          <div className="gauges">
            <ResourceGauge
              label="CPU"
              percent={resources.cpu_percent}
              detail={`${resources.cpu_cores} cœurs`}
              limit={
                config && config.status !== "Disabled"
                  ? config.cpu_limit_percent
                  : undefined
              }
              limitMax={100}
              limitStep={5}
              limitUnit="%"
              onLimitChange={
                config && config.status !== "Disabled"
                  ? (v) => setLimit("cpu", v)
                  : undefined
              }
            />
            <ResourceGauge
              label="RAM"
              percent={resources.ram_percent}
              detail={`${resources.ram_used_mb} / ${resources.ram_total_mb} Mo`}
              limit={
                config && config.status !== "Disabled"
                  ? config.ram_limit_mb
                  : undefined
              }
              limitMax={resources.ram_total_mb}
              limitStep={256}
              limitUnit="Mo"
              onLimitChange={
                config && config.status !== "Disabled"
                  ? (v) => setLimit("ram", v)
                  : undefined
              }
            />
            {resources.gpu_available && (
              <ResourceGauge
                label={`GPU (${resources.gpu_name})`}
                percent={resources.gpu_percent}
                detail={`${resources.gpu_memory_used_mb} / ${resources.gpu_memory_total_mb} Mo`}
                limit={
                  config && config.status !== "Disabled"
                    ? config.gpu_limit_percent
                    : undefined
                }
                limitMax={100}
                limitStep={5}
                limitUnit="%"
                onLimitChange={
                  config && config.status !== "Disabled"
                    ? (v) => setLimit("gpu", v)
                    : undefined
                }
              />
            )}
          </div>
        </section>
      )}

      {resources && tasks.length > 0 && (
        <section className="section">
          <h3>Répartition par utilisateur</h3>
          <UsageBreakdown
            tasks={tasks}
            totalCpuPercent={100}
            totalRamMb={resources.ram_total_mb}
            totalGpuPercent={100}
            gpuAvailable={resources.gpu_available}
          />
        </section>
      )}

      <section className="section">
        <h3>Environnement Python pour les tâches reçues</h3>
        <ManagedVenvPanel />
      </section>

      <section className="section">
        <h3>Qui utilise mes ressources ?</h3>
        <div className="concurrency-cap">
          <label className="concurrency-cap__label">
            Tâches simultanées maximum :
            <input
              type="number"
              min={1}
              max={64}
              value={maxConcurrent}
              onChange={(e) =>
                void handleConcurrencyChange(Number(e.target.value))
              }
              className="concurrency-cap__input"
            />
          </label>
          <p className="concurrency-cap__hint">
            Au-delà de cette limite, les tâches reçues attendent leur tour
            (statut « En attente »). Évite qu'un pair sature votre machine en
            envoyant 100 tâches d'un coup.
          </p>
        </div>
        <TaskList tasks={tasks} direction="incoming" onCancelled={refresh} />
      </section>

      <SecurityLogPanel />
    </div>
  );
}
