import type { SharingStatus } from "../lib/api";
import { useT } from "../lib/i18n";
import type { MessageKey } from "../lib/messages";

interface SharingToggleProps {
  status: SharingStatus;
  onEnable: () => void;
  onDisable: () => void;
  onPause: () => void;
  onResume: () => void;
}

const STATUS_LABEL_KEY: Record<SharingStatus, { key: MessageKey; className: string }> = {
  Disabled: { key: "sharing.status_disabled", className: "status--disabled" },
  Active: { key: "sharing.status_active", className: "status--active" },
  Paused: { key: "sharing.status_paused", className: "status--paused" },
};

export function SharingToggle({
  status,
  onEnable,
  onDisable,
  onPause,
  onResume,
}: SharingToggleProps) {
  const t = useT();
  const { key, className } = STATUS_LABEL_KEY[status];

  const handleDisable = () => {
    if (window.confirm(t("sharing.confirm_disable"))) {
      onDisable();
    }
  };

  return (
    <div className="sharing-toggle">
      <div className={`sharing-toggle__status ${className}`}>
        <span className="sharing-toggle__dot" />
        <span>{t(key)}</span>
      </div>
      <div className="sharing-toggle__actions">
        {status === "Disabled" && (
          <button className="btn btn--primary" onClick={onEnable}>
            {t("sharing.btn_enable")}
          </button>
        )}
        {status === "Active" && (
          <>
            <button
              className="btn btn--warning"
              onClick={onPause}
              title={t("sharing.tip_pause")}
            >
              {t("sharing.btn_pause")}
            </button>
            <button
              className="btn btn--danger"
              onClick={handleDisable}
              title={t("sharing.tip_disable")}
            >
              {t("sharing.btn_disable")}
            </button>
          </>
        )}
        {status === "Paused" && (
          <>
            <button className="btn btn--primary" onClick={onResume}>
              {t("sharing.btn_resume")}
            </button>
            <button
              className="btn btn--danger"
              onClick={handleDisable}
              title={t("sharing.tip_disable_paused")}
            >
              {t("sharing.btn_disable")}
            </button>
          </>
        )}
      </div>
    </div>
  );
}
