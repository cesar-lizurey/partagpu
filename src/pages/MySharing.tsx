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
  getResourceHistory,
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
import type {
  ResourceSample,
  ResourceUsage,
  SharingConfig,
  Task,
  UserStatus,
} from "../lib/api";
import { aggregateByUser, buildSelfUsage } from "../lib/usage";
import { Sparkline } from "../components/Sparkline";
import { useT } from "../lib/i18n";
import type { MessageKey } from "../lib/messages";

function UserSetup({
  userStatus,
  onDone,
}: {
  userStatus: UserStatus;
  onDone: () => void;
}) {
  const t = useT();
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (password.length < 4) {
      setError(t("user.err_too_short"));
      return;
    }
    if (password !== confirm) {
      setError(t("user.err_mismatch"));
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

  const statusKey: Record<string, MessageKey> = {
    Missing: "user.status_missing",
    NoLogin: "user.status_no_login",
    NoPassword: "user.status_no_password",
  };

  return (
    <div className="user-setup">
      <div className="user-setup__status">
        <span
          className={`user-setup__dot ${userStatus === "Ready" ? "user-setup__dot--ok" : "user-setup__dot--warn"}`}
        />
        <span>
          {userStatus === "Ready"
            ? t("user.status_ready")
            : statusKey[userStatus]
              ? t(statusKey[userStatus])
              : t("user.status_unknown")}
        </span>
      </div>

      {(userStatus === "NoPassword" || userStatus === "Ready") && (
        <form className="user-setup__form" onSubmit={handleSubmit}>
          <p className="user-setup__hint">
            {userStatus === "Ready" ? t("user.hint_modify") : t("user.hint_define")}
          </p>
          <div className="user-setup__fields">
            <input
              type="password"
              placeholder={t("user.password_placeholder")}
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="new-password"
            />
            <input
              type="password"
              placeholder={t("user.confirm_placeholder")}
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              autoComplete="new-password"
            />
            <button className="btn btn--primary" type="submit" disabled={loading}>
              {loading ? "..." : userStatus === "Ready" ? t("user.btn_modify") : t("user.btn_define")}
            </button>
          </div>
          {error && <p className="user-setup__error">{error}</p>}
        </form>
      )}
    </div>
  );
}

function HistoryPanel({
  label,
  unit,
  values,
  max,
  current,
  color,
  fill,
}: {
  label: string;
  unit: string;
  values: number[];
  max: number;
  current: number;
  color: string;
  fill: string;
}) {
  const t = useT();
  const peak = values.length > 0 ? Math.max(...values) : 0;
  const avg =
    values.length > 0
      ? values.reduce((s, v) => s + v, 0) / values.length
      : 0;
  return (
    <div className="history__panel">
      <div className="history__panel-header">
        <span className="history__panel-label">{label}</span>
        <span className="history__panel-current">
          {Math.round(current)} {unit}
        </span>
      </div>
      <Sparkline
        values={values}
        max={max}
        height={50}
        color={color}
        fillColor={fill}
        ariaLabel={t("mysharing.history_aria", { label })}
      />
      <div className="history__panel-stats">
        <span>{t("mysharing.history_avg", { value: Math.round(avg), unit })}</span>
        <span>{t("mysharing.history_peak", { value: Math.round(peak), unit })}</span>
      </div>
    </div>
  );
}

export function MySharing() {
  const t = useT();
  const [resources, setResources] = useState<ResourceUsage | null>(null);
  const [history, setHistory] = useState<ResourceSample[]>([]);
  const [config, setConfig] = useState<SharingConfig | null>(null);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [userStatus, setUserStatus] = useState<UserStatus>("Missing");
  const [maxConcurrent, setMaxConcurrent] = useState<number>(4);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [res, hist, cfg, ts, us, mc] = await Promise.all([
        getResources(),
        getResourceHistory(),
        getSharingConfig(),
        getIncomingTasks(),
        getUserStatus(),
        getMaxConcurrentTasks(),
      ]);
      setResources(res);
      setHistory(hist);
      setConfig(cfg);
      setTasks(ts);
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
      <h2>{t("mysharing.title")}</h2>
      <p className="page__subtitle">{t("mysharing.subtitle")}</p>

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
          <h3>{t("mysharing.account_section")}</h3>
          <UserSetup userStatus={userStatus} onDone={refresh} />
        </section>
      )}

      {resources && (
        <section className="section">
          <h3>{t("mysharing.resources_section")}</h3>
          {config && config.status !== "Disabled" && (
            <p className="section__hint">{t("mysharing.resources_hint")}</p>
          )}
          {(() => {
            // Calcule les segments par utilisateur sur chaque jauge :
            // - segment vert = vous (ce qui n'est pas attribuable a une tache PartaGPU)
            // - un segment couleur par utilisateur distant qui consomme la machine
            // En mode CPU/GPU les valeurs sont des % cumulables ; pour la RAM
            // on convertit les Mo de chaque tache en % du total physique.
            const remoteUsers = aggregateByUser(tasks, t("common.unknown"));
            const remoteCpu = remoteUsers.reduce((s, u) => s + u.cpu, 0);
            const remoteRam = remoteUsers.reduce((s, u) => s + u.ramMb, 0);
            const remoteGpu = remoteUsers.reduce((s, u) => s + u.gpu, 0);
            // On harmonise le total affiche avec ce que la barre montre vraiment :
            // les taches sont comptees via cgroup memory.current (inclut le cache),
            // alors que sysinfo::used_memory() exclut le cache. Si une tache charge
            // un dataset en cache, sa mesure cgroup peut depasser le "used" systeme.
            // Sans cette reconciliation, le badge disait 4 Go pendant que la barre
            // remplissait 14 Go, et le segment "vous" vert disparaissait car
            // max(0, sysUsed - remoteRam) tombait a 0.
            const displayedCpu = Math.max(resources.cpu_percent, remoteCpu);
            const displayedRamMb = Math.max(resources.ram_used_mb, remoteRam);
            const displayedGpu = Math.max(resources.gpu_percent, remoteGpu);
            const displayedRamPercent =
              resources.ram_total_mb > 0
                ? (displayedRamMb / resources.ram_total_mb) * 100
                : 0;
            const selfLabel = t("gauge.you_label");
            const self = buildSelfUsage(
              displayedCpu,
              displayedRamMb,
              displayedGpu,
              remoteUsers,
              selfLabel,
            );
            const all = self ? [self, ...remoteUsers] : remoteUsers;
            const cpuSegments = all.map((u) => ({
              value: u.cpu,
              color: u.color,
              name: u.name,
            }));
            const ramSegments =
              resources.ram_total_mb > 0
                ? all.map((u) => ({
                    value: (u.ramMb / resources.ram_total_mb) * 100,
                    color: u.color,
                    name: u.name,
                  }))
                : [];
            const gpuSegments = all.map((u) => ({
              value: u.gpu,
              color: u.color,
              name: u.name,
            }));
            return (
              <div className="gauges">
                <ResourceGauge
                  label="CPU"
                  percent={displayedCpu}
                  detail={t("mysharing.cores_suffix", { n: resources.cpu_cores })}
                  segments={cpuSegments}
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
                  percent={displayedRamPercent}
                  detail={`${displayedRamMb} / ${resources.ram_total_mb} Mo`}
                  segments={ramSegments}
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
                    percent={displayedGpu}
                    detail={`${resources.gpu_memory_used_mb} / ${resources.gpu_memory_total_mb} Mo`}
                    segments={gpuSegments}
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
                    limitWarning={
                      config &&
                      config.status !== "Disabled" &&
                      config.gpu_limit_percent < 100 &&
                      !resources.gpu_limit_enforced
                        ? t("gauge.gpu_advisory")
                        : undefined
                    }
                  />
                )}
              </div>
            );
          })()}

          {history.length > 1 && (
            <div className="history">
              <h4 className="history__title">
                {t("mysharing.history_title")}
                <span className="history__sublabel">
                  {t("mysharing.history_sublabel")}
                </span>
              </h4>
              <div className="history__row">
                <HistoryPanel
                  label="CPU"
                  unit="%"
                  values={history.map((s) => s.cpu_percent)}
                  max={100}
                  current={resources.cpu_percent}
                  color="#6366f1"
                  fill="rgba(99,102,241,0.18)"
                />
                <HistoryPanel
                  label="RAM"
                  unit="Mo"
                  values={history.map((s) => s.ram_used_mb)}
                  max={resources.ram_total_mb || 1}
                  current={resources.ram_used_mb}
                  color="#10b981"
                  fill="rgba(16,185,129,0.18)"
                />
                {resources.gpu_available && (
                  <HistoryPanel
                    label="GPU"
                    unit="%"
                    values={history.map((s) => s.gpu_percent)}
                    max={100}
                    current={resources.gpu_percent}
                    color="#f59e0b"
                    fill="rgba(245,158,11,0.18)"
                  />
                )}
              </div>
            </div>
          )}
        </section>
      )}

      {resources && tasks.length > 0 && (
        <section className="section">
          <h3>{t("mysharing.breakdown_section")}</h3>
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
        <h3>{t("mysharing.python_section")}</h3>
        <ManagedVenvPanel />
      </section>

      <section className="section">
        <h3>{t("mysharing.who_section")}</h3>
        <div className="concurrency-cap">
          <label className="concurrency-cap__label">
            {t("mysharing.concurrency_label")}
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
            {t("mysharing.concurrency_hint")}
          </p>
        </div>
        <TaskList tasks={tasks} direction="incoming" onCancelled={refresh} />
      </section>

      <SecurityLogPanel />
    </div>
  );
}
