import { useEffect, useRef } from "react";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import type { Task, TaskStatus } from "./api";

const TERMINAL: ReadonlySet<TaskStatus> = new Set([
  "Completed",
  "Failed",
  "Cancelled",
]);

let permissionPromise: Promise<boolean> | null = null;

/** Lazily ask the OS for desktop notification permission, once per session.
 *  Subsequent callers reuse the same promise so we never trigger two prompts. */
async function ensurePermission(): Promise<boolean> {
  if (permissionPromise) return permissionPromise;
  permissionPromise = (async () => {
    if (await isPermissionGranted()) return true;
    const decision = await requestPermission();
    return decision === "granted";
  })();
  return permissionPromise;
}

interface NotifyParams {
  title: string;
  body: string;
}

async function notify({ title, body }: NotifyParams): Promise<void> {
  if (!(await ensurePermission())) return;
  try {
    sendNotification({ title, body });
  } catch {
    // Notification API can fail on environments without a notification
    // daemon (headless, some minimal WMs). Failing silently is fine —
    // the user still sees the result in the UI.
  }
}

/** Fire a desktop notification whenever an outgoing task transitions
 *  from non-terminal to a terminal state (Completed / Failed / Cancelled).
 *  Useful when the user is away from the window during a long DDP run.
 *
 *  The hook tracks the previous status per task id internally — no need
 *  to keep state in the caller. Only fires for tasks already known on
 *  a previous render, so the initial fetch (which may include historical
 *  Completed tasks) doesn't spam the user. */
export function useTaskCompletionNotifications(tasks: Task[]): void {
  // Map<task_id, last seen status>. Persists across renders.
  const prevStatuses = useRef<Map<string, TaskStatus>>(new Map());

  useEffect(() => {
    const prev = prevStatuses.current;
    for (const task of tasks) {
      const before = prev.get(task.id);
      // Only notify on a *transition* into terminal — skip the first time
      // we see a task to avoid flooding on initial load.
      if (
        before !== undefined &&
        !TERMINAL.has(before) &&
        TERMINAL.has(task.status)
      ) {
        const cmd = task.command.length > 60
          ? task.command.slice(0, 60) + "…"
          : task.command;
        notify({
          title: titleFor(task),
          body: `${task.target_machine} — ${cmd}`,
        });
      }
      prev.set(task.id, task.status);
    }
    // Garbage-collect ids that disappeared (task removed from list).
    const live = new Set(tasks.map((t) => t.id));
    for (const id of [...prev.keys()]) {
      if (!live.has(id)) prev.delete(id);
    }
  }, [tasks]);
}

function titleFor(task: Task): string {
  switch (task.status) {
    case "Completed":
      return "✅ Tâche terminée";
    case "Failed":
      return `❌ Tâche échouée${task.exit_code != null ? ` (exit ${task.exit_code})` : ""}`;
    case "Cancelled":
      return "⏹ Tâche annulée";
    default:
      return "Tâche terminée";
  }
}
