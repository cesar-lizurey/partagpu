import { useEffect, useState, useCallback } from "react";
import { ResourceGauge } from "../components/ResourceGauge";
import { ResourceSliders } from "../components/ResourceSliders";
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
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [res, cfg, t, us] = await Promise.all([
        getResources(),
        getSharingConfig(),
        getIncomingTasks(),
        getUserStatus(),
      ]);
      setResources(res);
      setConfig(cfg);
      setTasks(t);
      setUserStatus(us);
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
          <div className="gauges">
            <ResourceGauge
              label="CPU"
              percent={resources.cpu_percent}
              detail={`${resources.cpu_cores} cœurs`}
              limit={config?.cpu_limit_percent}
            />
            <ResourceGauge
              label="RAM"
              percent={resources.ram_percent}
              detail={`${resources.ram_used_mb} / ${resources.ram_total_mb} Mo`}
            />
            {resources.gpu_available && (
              <ResourceGauge
                label={`GPU (${resources.gpu_name})`}
                percent={resources.gpu_percent}
                detail={`${resources.gpu_memory_used_mb} / ${resources.gpu_memory_total_mb} Mo`}
                limit={config?.gpu_limit_percent}
              />
            )}
          </div>
        </section>
      )}

      {config && config.status !== "Disabled" && resources && (
        <section className="section">
          <ResourceSliders
            cpuLimit={config.cpu_limit_percent}
            ramLimitMb={config.ram_limit_mb}
            gpuLimit={config.gpu_limit_percent}
            ramTotalMb={resources.ram_total_mb}
            gpuAvailable={resources.gpu_available}
            onChange={handleLimitsChange}
          />
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
        <h3>Qui utilise mes ressources ?</h3>
        <TaskList tasks={tasks} direction="incoming" />
      </section>

      <SecurityLogPanel />
    </div>
  );
}
