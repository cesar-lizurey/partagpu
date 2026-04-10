import { invoke } from "@tauri-apps/api/core";

// ── Types ──────────────────────────────────────────────────

export interface Peer {
  id: string;
  display_name: string;
  hostname: string;
  ip: string;
  port: number;
  sharing_enabled: boolean;
  cpu_limit: number;
  ram_limit: number;
  gpu_limit: number;
  totp_code: string;
  verified: boolean;
  hostname_conflict: boolean;
}

export interface RoomStatus {
  joined: boolean;
  room_name: string;
  passphrase: string;
}

export interface CreateRoomResult {
  passphrase: string;
  secret_base32: string;
}

export interface ResourceUsage {
  cpu_percent: number;
  cpu_cores: number;
  ram_used_mb: number;
  ram_total_mb: number;
  ram_percent: number;
  gpu_percent: number;
  gpu_name: string;
  gpu_memory_used_mb: number;
  gpu_memory_total_mb: number;
  gpu_available: boolean;
}

export type SharingStatus = "Disabled" | "Active" | "Paused";

export interface SharingConfig {
  status: SharingStatus;
  cpu_limit_percent: number;
  ram_limit_mb: number;
  gpu_limit_percent: number;
}

export type TaskStatus =
  | "Queued"
  | "Running"
  | "Completed"
  | "Failed"
  | "Cancelled";

export interface Task {
  id: string;
  command: string;
  args: string[];
  source_machine: string;
  source_user: string;
  target_machine: string;
  status: TaskStatus;
  progress: number;
  cpu_usage: number;
  ram_usage_mb: number;
  gpu_usage: number;
  output: string;
  error_output: string;
  exit_code: number | null;
  created_at: number;
}

export interface MachineInfo {
  hostname: string;
  ip: string;
  user: string;
  display_name: string;
}

export type UserStatus = "Missing" | "NoLogin" | "NoPassword" | "Ready";

// ── API calls ──────────────────────────────────────────────

export const getUserStatus = () => invoke<UserStatus>("get_user_status");

export const setUserPassword = (password: string) =>
  invoke<string>("set_user_password", { password });

export const getPeers = () => invoke<Peer[]>("get_peers");

export const getResources = () => invoke<ResourceUsage>("get_resources");

export const getSharingConfig = () =>
  invoke<SharingConfig>("get_sharing_config");

export const enableSharing = () => invoke<SharingConfig>("enable_sharing");

export const disableSharing = () => invoke<SharingConfig>("disable_sharing");

export const pauseSharing = () => invoke<SharingConfig>("pause_sharing");

export const resumeSharing = () => invoke<SharingConfig>("resume_sharing");

export const setSharingLimits = (
  cpuPercent: number,
  ramLimitMb: number,
  gpuPercent: number,
) =>
  invoke<SharingConfig>("set_sharing_limits", {
    cpuPercent,
    ramLimitMb,
    gpuPercent,
  });

export const getIncomingTasks = () => invoke<Task[]>("get_incoming_tasks");

export const getOutgoingTasks = () => invoke<Task[]>("get_outgoing_tasks");

export const submitTask = (
  args: string[],
  sourceMachine: string,
  sourceUser: string,
  timeoutSecs?: number,
) =>
  invoke<Task>("submit_task", { args, sourceMachine, sourceUser, timeoutSecs });

export const getAllowlist = () => invoke<string[]>("get_allowlist");

export const addToAllowlist = (command: string) =>
  invoke<void>("add_to_allowlist", { command });

export const removeFromAllowlist = (command: string) =>
  invoke<void>("remove_from_allowlist", { command });

export const checkSandboxAvailable = () =>
  invoke<boolean>("check_sandbox_available");

// ── Security log ──────────────────────────────────────────

export interface SecurityEvent {
  timestamp: number;
  level: "Info" | "Warning" | "Alert";
  category: string;
  message: string;
  source_ip: string | null;
  source_host: string | null;
}

export const getSecurityLog = (since?: number) =>
  invoke<SecurityEvent[]>("get_security_log", { since: since ?? null });

export const clearSecurityLog = () => invoke<void>("clear_security_log");

// ── Room / TOTP auth ──────────────────────────────────────

export const createRoom = (roomName: string) =>
  invoke<CreateRoomResult>("create_room", { roomName });

export const joinRoom = (roomName: string, passphrase: string) =>
  invoke<void>("join_room", { roomName, passphrase });

export const leaveRoom = () => invoke<void>("leave_room");

export const getRoomStatus = () => invoke<RoomStatus>("get_room_status");

export const getRoomSecret = () => invoke<string | null>("get_room_secret");

// ── Discovery ─────────────────────────────────────────────

export const getDisplayName = () => invoke<string>("get_display_name");

export const setDisplayName = (name: string) =>
  invoke<string>("set_display_name", { name });

export const getMachineInfo = () => invoke<MachineInfo>("get_machine_info");
