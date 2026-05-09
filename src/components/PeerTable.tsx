import type { Peer } from "../lib/api";
import { useT } from "../lib/i18n";

interface PeerTableProps {
  peers: Peer[];
  emptyMessage?: string;
}

function peerLabel(peer: Peer): string {
  if (peer.display_name && peer.display_name !== peer.hostname) {
    return `${peer.display_name} (${peer.hostname})`;
  }
  return peer.hostname;
}

function rowClass(peer: Peer): string {
  if (peer.hostname_conflict) return "peer-table__row--conflict";
  if (!peer.verified) return "peer-table__row--unverified";
  return "";
}

export function PeerTable({ peers, emptyMessage }: PeerTableProps) {
  const t = useT();

  const authBadge = (peer: Peer) => {
    if (peer.hostname_conflict) {
      return (
        <span className="badge badge--failed" title={t("peers.badge_conflict_title")}>
          !!
        </span>
      );
    }
    if (!peer.verified) {
      return (
        <span className="badge badge--failed" title={t("peers.badge_unverified_title")}>
          ?
        </span>
      );
    }
    return (
      <span className="badge badge--completed" title={t("peers.badge_verified_title")}>
        OK
      </span>
    );
  };

  if (peers.length === 0) {
    return <p className="empty-state">{emptyMessage ?? t("peers.empty_default")}</p>;
  }

  const unverifiedCount = peers.filter((p) => !p.verified).length;
  const conflictCount = peers.filter((p) => p.hostname_conflict).length;

  return (
    <>
      {conflictCount > 0 && (
        <div className="alert alert--error">
          {conflictCount === 1
            ? t("peers.conflict_alert_one")
            : t("peers.conflict_alert_many", { n: conflictCount })}
        </div>
      )}
      {unverifiedCount > 0 && conflictCount === 0 && (
        <div className="alert alert--warning">
          {unverifiedCount === 1
            ? t("peers.unverified_alert_one")
            : t("peers.unverified_alert_many", { n: unverifiedCount })}
        </div>
      )}
      <table className="peer-table">
        <thead>
          <tr>
            <th>{t("peers.col_machine")}</th>
            <th>{t("peers.col_ip")}</th>
            <th>{t("peers.col_auth")}</th>
            <th>{t("peers.col_sharing")}</th>
            <th>{t("peers.col_cpu")}</th>
            <th>{t("peers.col_ram")}</th>
            <th>{t("peers.col_gpu")}</th>
          </tr>
        </thead>
        <tbody>
          {peers.map((peer) => (
            <tr key={peer.id} className={rowClass(peer)}>
              <td className="peer-table__hostname">
                {peerLabel(peer)}
                {peer.hostname_conflict && (
                  <span className="peer-table__conflict-icon" title={t("peers.conflict_icon_title")}>
                    {" "}!!
                  </span>
                )}
              </td>
              <td className="peer-table__ip">{peer.ip}</td>
              <td>{authBadge(peer)}</td>
              <td>
                <span
                  className={`badge ${peer.sharing_enabled ? "badge--active" : "badge--disabled"}`}
                >
                  {peer.sharing_enabled
                    ? t("peers.sharing_active")
                    : t("peers.sharing_inactive")}
                </span>
              </td>
              <td>{peer.cpu_limit}%</td>
              <td>{peer.ram_limit > 0 ? `${peer.ram_limit} Mo` : t("common.none_dash")}</td>
              <td>{peer.gpu_limit}%</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
