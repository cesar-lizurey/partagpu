import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import {
  getManagedVenvStatus,
  removeManagedVenv,
  setupManagedVenv,
  type ManagedVenvStatus,
} from "../lib/api";
import { useT } from "../lib/i18n";

/** Maximum number of log lines kept in memory while the helper streams. */
const MAX_LOG_LINES = 500;

/**
 * Panel for the managed Python venv (provides torch + numpy to the sandbox so
 * peers don't have to `sudo pip install --break-system-packages` themselves).
 */
export function ManagedVenvPanel() {
  const t = useT();
  const [status, setStatus] = useState<ManagedVenvStatus | null>(null);
  const [busy, setBusy] = useState<"install" | "remove" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [installLog, setInstallLog] = useState<string[]>([]);
  const logBoxRef = useRef<HTMLPreElement | null>(null);

  const refresh = async () => {
    try {
      setStatus(await getManagedVenvStatus());
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  // Listen to helper output events (stdout + stderr lines streamed by the
  // backend during long-running installs). Append to the log buffer with
  // a hard cap so a chatty pip install doesn't blow up memory.
  useEffect(() => {
    let unlistenStdout: UnlistenFn | null = null;
    let unlistenStderr: UnlistenFn | null = null;
    (async () => {
      unlistenStdout = await listen<string>("helper-output", (e) => {
        setInstallLog((prev) => {
          const next = [...prev, e.payload];
          return next.length > MAX_LOG_LINES
            ? next.slice(next.length - MAX_LOG_LINES)
            : next;
        });
      });
      unlistenStderr = await listen<string>("helper-output-err", (e) => {
        setInstallLog((prev) => {
          const next = [...prev, `[stderr] ${e.payload}`];
          return next.length > MAX_LOG_LINES
            ? next.slice(next.length - MAX_LOG_LINES)
            : next;
        });
      });
    })();
    return () => {
      unlistenStdout?.();
      unlistenStderr?.();
    };
  }, []);

  // Auto-scroll the log to the bottom as new lines arrive.
  useEffect(() => {
    if (logBoxRef.current) {
      logBoxRef.current.scrollTop = logBoxRef.current.scrollHeight;
    }
  }, [installLog]);

  const handleInstall = async () => {
    const isUpdate = status?.installed === true;
    const message = isUpdate ? t("venv.confirm_update") : t("venv.confirm_install");
    if (!confirm(message)) {
      return;
    }
    setError(null);
    setInstallLog([]);
    setBusy("install");
    try {
      await setupManagedVenv();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const handleRemove = async () => {
    if (!confirm(t("venv.confirm_remove"))) {
      return;
    }
    setError(null);
    setBusy("remove");
    try {
      await removeManagedVenv();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  if (!status) {
    return <p className="empty-state">{t("common.loading")}</p>;
  }

  return (
    <div className="managed-venv">
      <p className="managed-venv__intro">
        {t("venv.intro_p1")}
        <code>import torch</code>
        {t("venv.intro_p2")}
        <code>sudo pip install</code>
        {t("venv.intro_p3")}
        <code>{status.path}</code>
        {t("venv.intro_p4")}
        <strong>{t("venv.intro_p5")}</strong>
        {" : "}
        <code>torch</code>, <code>torchvision</code>, <code>numpy</code>,{" "}
        <code>scipy</code>, <code>pandas</code>, <code>scikit-learn</code>,{" "}
        <code>matplotlib</code>, <code>pillow</code>
        {t("venv.intro_p6")}
        <code>python3</code>
        {t("venv.intro_p7")}
      </p>

      <div className="managed-venv__status">
        {status.installed ? (
          <>
            <span className="badge badge--completed">{t("venv.installed")}</span>
            <span className="managed-venv__path">
              <code>{status.path}</code>
            </span>
          </>
        ) : (
          <>
            <span className="badge badge--disabled">{t("venv.not_installed")}</span>
            <span className="managed-venv__hint">
              {t("venv.not_installed_hint_p1")}
              <code>sudo pip install --break-system-packages …</code>
              {t("venv.not_installed_hint_p2")}
            </span>
          </>
        )}
      </div>

      <div className="managed-venv__actions">
        {status.installed ? (
          <>
            <button
              type="button"
              onClick={handleInstall}
              disabled={busy !== null}
              className="btn btn--secondary"
              title={t("venv.btn_check_updates_title")}
            >
              {busy === "install"
                ? t("venv.btn_check_updates_progress")
                : t("venv.btn_check_updates")}
            </button>
            <button
              type="button"
              onClick={handleRemove}
              disabled={busy !== null}
              className="btn btn--danger"
            >
              {busy === "remove" ? t("venv.btn_remove_progress") : t("venv.btn_remove")}
            </button>
          </>
        ) : (
          <button
            type="button"
            onClick={handleInstall}
            disabled={busy !== null}
            className="btn btn--primary"
          >
            {busy === "install"
              ? t("venv.btn_install_progress")
              : t("venv.btn_install")}
          </button>
        )}
      </div>

      {busy === "install" ? (
        <p className="managed-venv__progress">
          {t("venv.installing_msg")}
        </p>
      ) : null}

      {(busy === "install" || installLog.length > 0) && (
        <details
          className="managed-venv__log-box"
          open={busy === "install"}
        >
          <summary>
            {t(
              installLog.length === 1
                ? "venv.log_summary_one"
                : "venv.log_summary_many",
              { n: installLog.length },
            )}
          </summary>
          <pre ref={logBoxRef} className="managed-venv__log">
            {installLog.length > 0
              ? installLog.join("\n")
              : t("venv.log_waiting")}
          </pre>
        </details>
      )}

      {error ? <div className="alert alert--error">{error}</div> : null}
    </div>
  );
}
