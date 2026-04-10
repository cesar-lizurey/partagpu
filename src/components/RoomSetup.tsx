import { useEffect, useState, useCallback } from "react";
import {
  createRoom,
  joinRoom,
  leaveRoom,
  getRoomStatus,
} from "../lib/api";
import type { RoomStatus } from "../lib/api";

export function RoomSetup() {
  const [status, setStatus] = useState<RoomStatus | null>(null);
  const [mode, setMode] = useState<"idle" | "create" | "join">("idle");
  const [roomName, setRoomName] = useState("");
  const [joinPassphrase, setJoinPassphrase] = useState("");
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const s = await getRoomStatus();
      setStatus(s);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, 1000);
    return () => clearInterval(interval);
  }, [refresh]);

  const handleCreate = async () => {
    setError(null);
    if (!roomName.trim()) {
      setError("Entrez un nom de salle.");
      return;
    }
    try {
      await createRoom(roomName.trim());
      setMode("idle");
      refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleJoin = async () => {
    setError(null);
    if (!roomName.trim()) {
      setError("Entrez un nom de salle.");
      return;
    }
    if (!joinPassphrase.trim()) {
      setError("Entrez le code d'accès.");
      return;
    }
    try {
      await joinRoom(roomName.trim(), joinPassphrase.trim());
      setMode("idle");
      refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleLeave = async () => {
    await leaveRoom();
    refresh();
  };

  // ── Not joined ─────────────────────────────────────────

  if (!status?.joined) {
    return (
      <div className="room-setup">
        <div className="room-setup__header">
          <span className="room-setup__dot room-setup__dot--off" />
          <span>Aucune salle configurée</span>
        </div>

        {mode === "idle" && (
          <div className="room-setup__actions">
            <button
              className="btn btn--primary"
              onClick={() => setMode("create")}
            >
              Créer une salle
            </button>
            <button
              className="btn btn--secondary"
              onClick={() => setMode("join")}
            >
              Rejoindre une salle
            </button>
          </div>
        )}

        {mode === "create" && (
          <div className="room-setup__form">
            <input
              type="text"
              placeholder="Nom de la salle (ex: Salle B204)"
              value={roomName}
              onChange={(e) => setRoomName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && handleCreate()}
            />
            <div className="room-setup__form-actions">
              <button className="btn btn--primary" onClick={handleCreate}>
                Créer
              </button>
              <button
                className="btn btn--danger"
                onClick={() => setMode("idle")}
              >
                Annuler
              </button>
            </div>
          </div>
        )}

        {mode === "join" && (
          <div className="room-setup__form">
            <input
              type="text"
              placeholder="Nom de la salle"
              value={roomName}
              onChange={(e) => setRoomName(e.target.value)}
            />
            <input
              type="text"
              placeholder="Code d'accès (ex: pomme-tigre-bleu-ocean)"
              value={joinPassphrase}
              onChange={(e) => setJoinPassphrase(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && handleJoin()}
              className="room-setup__passphrase-input"
            />
            <p className="room-setup__form-hint">
              Demandez le code d'accès au camarade qui a créé la salle.
            </p>
            <div className="room-setup__form-actions">
              <button className="btn btn--primary" onClick={handleJoin}>
                Rejoindre
              </button>
              <button
                className="btn btn--danger"
                onClick={() => setMode("idle")}
              >
                Annuler
              </button>
            </div>
          </div>
        )}

        {error && <p className="room-setup__error">{error}</p>}
      </div>
    );
  }

  // ── Joined ─────────────────────────────────────────────

  return (
    <div className="room-setup">
      <div className="room-setup__header">
        <span className="room-setup__dot room-setup__dot--on" />
        <span>
          Salle <strong>{status.room_name}</strong>
        </span>
        <button className="btn btn--danger btn--small" onClick={handleLeave}>
          Quitter
        </button>
      </div>

      <div className="room-setup__connected">
        <div className="room-setup__passphrase-section">
          <p className="room-setup__hint">
            Dictez ce code d'accès aux camarades pour qu'ils rejoignent :
          </p>
          <div className="room-setup__passphrase">
            {status.passphrase}
          </div>
        </div>
      </div>

      {error && <p className="room-setup__error">{error}</p>}
    </div>
  );
}
