import type { Peer } from "../lib/api";

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

function authBadge(peer: Peer) {
  if (peer.hostname_conflict) {
    return (
      <span className="badge badge--failed" title="Conflit de hostname — possible usurpation">
        !!
      </span>
    );
  }
  if (!peer.verified) {
    return (
      <span className="badge badge--failed" title="Non vérifié">
        ?
      </span>
    );
  }
  return (
    <span className="badge badge--completed" title="TOTP vérifié">
      OK
    </span>
  );
}

export function PeerTable({
  peers,
  emptyMessage = "Aucune machine détectée sur le réseau.",
}: PeerTableProps) {
  if (peers.length === 0) {
    return <p className="empty-state">{emptyMessage}</p>;
  }

  const unverifiedCount = peers.filter((p) => !p.verified).length;
  const conflictCount = peers.filter((p) => p.hostname_conflict).length;

  return (
    <>
      {conflictCount > 0 && (
        <div className="alert alert--error">
          Conflit de hostname détecté — {conflictCount} machine
          {conflictCount > 1 ? "s" : ""} utilise
          {conflictCount > 1 ? "nt" : ""} un nom déjà pris par une autre IP.
          Cela peut indiquer une tentative d'usurpation d'identité.
        </div>
      )}
      {unverifiedCount > 0 && conflictCount === 0 && (
        <div className="alert alert--warning">
          {unverifiedCount} machine{unverifiedCount > 1 ? "s" : ""} non
          vérifiée{unverifiedCount > 1 ? "s" : ""} — les tâches provenant de
          ces postes seront refusées.
        </div>
      )}
      <table className="peer-table">
        <thead>
          <tr>
            <th>Machine</th>
            <th>IP</th>
            <th>Auth</th>
            <th>Partage</th>
            <th>CPU</th>
            <th>RAM</th>
            <th>GPU</th>
          </tr>
        </thead>
        <tbody>
          {peers.map((peer) => (
            <tr key={peer.id} className={rowClass(peer)}>
              <td className="peer-table__hostname">
                {peerLabel(peer)}
                {peer.hostname_conflict && (
                  <span className="peer-table__conflict-icon" title="Conflit de hostname">
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
                  {peer.sharing_enabled ? "Actif" : "Inactif"}
                </span>
              </td>
              <td>{peer.cpu_limit}%</td>
              <td>{peer.ram_limit > 0 ? `${peer.ram_limit} Mo` : "—"}</td>
              <td>{peer.gpu_limit}%</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
