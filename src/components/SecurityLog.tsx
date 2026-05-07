import { useEffect, useState, useCallback } from "react";
import { getSecurityLog, clearSecurityLog } from "../lib/api";
import type { SecurityEvent } from "../lib/api";
import { useI18n } from "../lib/i18n";
import type { MessageKey } from "../lib/messages";

const LEVEL_CLASS: Record<string, string> = {
  Info: "seclog__level--info",
  Warning: "seclog__level--warning",
  Alert: "seclog__level--alert",
};

const LEVEL_KEY: Record<string, MessageKey> = {
  Info: "seclog.level_info",
  Warning: "seclog.level_warning",
  Alert: "seclog.level_alert",
};

function formatTime(timestamp: number, lang: "fr" | "en"): string {
  const d = new Date(timestamp * 1000);
  return d.toLocaleTimeString(lang === "fr" ? "fr-FR" : "en-GB", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function SecurityLogPanel() {
  const { t, lang } = useI18n();
  const [events, setEvents] = useState<SecurityEvent[]>([]);
  const [expanded, setExpanded] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const evts = await getSecurityLog();
      setEvents(evts);
    } catch {
      // ignore
    }
  }, []);

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, 3000);
    return () => clearInterval(interval);
  }, [refresh]);

  const handleClear = async () => {
    await clearSecurityLog();
    setEvents([]);
  };

  const alertCount = events.filter((e) => e.level === "Alert").length;
  const warnCount = events.filter((e) => e.level === "Warning").length;

  return (
    <div className="seclog">
      <div className="seclog__header" onClick={() => setExpanded(!expanded)}>
        <span className="seclog__title">
          {t("seclog.title")}
          {alertCount > 0 && (
            <span className="seclog__badge seclog__badge--alert">
              {alertCount}
            </span>
          )}
          {warnCount > 0 && (
            <span className="seclog__badge seclog__badge--warning">
              {warnCount}
            </span>
          )}
          {alertCount === 0 && warnCount === 0 && events.length > 0 && (
            <span className="seclog__badge seclog__badge--info">
              {events.length}
            </span>
          )}
        </span>
        <span className="seclog__toggle">{expanded ? t("seclog.hide") : t("seclog.show")}</span>
      </div>

      {expanded && (
        <div className="seclog__body">
          {events.length === 0 ? (
            <p className="empty-state">{t("seclog.empty")}</p>
          ) : (
            <>
              <div className="seclog__actions">
                <button className="btn btn--secondary btn--small" onClick={handleClear}>
                  {t("seclog.clear")}
                </button>
              </div>
              <div className="seclog__list">
                {[...events].reverse().map((evt, i) => (
                  <div key={i} className="seclog__event">
                    <span className="seclog__time">{formatTime(evt.timestamp, lang)}</span>
                    <span className={`seclog__level ${LEVEL_CLASS[evt.level] || ""}`}>
                      {t(LEVEL_KEY[evt.level] ?? "seclog.level_info")}
                    </span>
                    <span className="seclog__message">{evt.message}</span>
                    {evt.source_ip && (
                      <span className="seclog__source">{evt.source_ip}</span>
                    )}
                  </div>
                ))}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
