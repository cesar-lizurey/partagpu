import { useEffect, useState, useCallback } from "react";
import {
  createRoom,
  joinRoom,
  leaveRoom,
  getRoomStatus,
} from "../lib/api";
import type { RoomStatus } from "../lib/api";
import { RevealOnHold } from "./RevealOnHold";
import { useT } from "../lib/i18n";

export function RoomSetup() {
  const t = useT();
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
      setError(t("room.err_need_name"));
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
      setError(t("room.err_need_name"));
      return;
    }
    if (!joinPassphrase.trim()) {
      setError(t("room.err_need_passphrase"));
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
          <span>{t("room.no_room")}</span>
        </div>

        {mode === "idle" && (
          <div className="room-setup__actions">
            <button
              className="btn btn--primary"
              onClick={() => setMode("create")}
            >
              {t("room.create")}
            </button>
            <button
              className="btn btn--secondary"
              onClick={() => setMode("join")}
            >
              {t("room.join")}
            </button>
          </div>
        )}

        {mode === "create" && (
          <div className="room-setup__form">
            <input
              type="text"
              placeholder={t("room.create_placeholder")}
              value={roomName}
              onChange={(e) => setRoomName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && handleCreate()}
            />
            <div className="room-setup__form-actions">
              <button className="btn btn--primary" onClick={handleCreate}>
                {t("room.create_btn")}
              </button>
              <button
                className="btn btn--danger"
                onClick={() => setMode("idle")}
              >
                {t("common.cancel")}
              </button>
            </div>
          </div>
        )}

        {mode === "join" && (
          <div className="room-setup__form">
            <input
              type="text"
              placeholder={t("room.join_name_placeholder")}
              value={roomName}
              onChange={(e) => setRoomName(e.target.value)}
            />
            <input
              type="text"
              placeholder={t("room.join_pass_placeholder")}
              value={joinPassphrase}
              onChange={(e) => setJoinPassphrase(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && handleJoin()}
              className="room-setup__passphrase-input"
            />
            <p className="room-setup__form-hint">
              {t("room.join_hint")}
            </p>
            <div className="room-setup__form-actions">
              <button className="btn btn--primary" onClick={handleJoin}>
                {t("room.join_btn")}
              </button>
              <button
                className="btn btn--danger"
                onClick={() => setMode("idle")}
              >
                {t("common.cancel")}
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
          {t("room.in_room_label")} <strong>{status.room_name}</strong>
        </span>
        <button className="btn btn--danger btn--small" onClick={handleLeave}>
          {t("room.leave")}
        </button>
      </div>

      <div className="room-setup__connected">
        <div className="room-setup__passphrase-section">
          <p className="room-setup__hint">
            {t("room.share_hint")}
          </p>
          <div className="room-setup__passphrase">
            <RevealOnHold value={status.passphrase} />
          </div>
        </div>
      </div>

      {error && <p className="room-setup__error">{error}</p>}
    </div>
  );
}
