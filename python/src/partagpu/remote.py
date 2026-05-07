"""Run a single command on a remote PartaGPU peer.

This is the foundation for distributed compute over a PartaGPU room. The local
PartaGPU app is the broker — your code talks to ``http://localhost:7654`` and
the app forwards the task to the chosen peer (signed with an HMAC keyed by
the shared room secret), runs it inside the peer's sandbox, and streams the
result back.

Typical use::

    import partagpu

    gpus = partagpu.discover()
    peer = next(g for g in gpus if g.host != "local")

    result = partagpu.run_remote(peer, [
        "python3", "-c",
        "import torch; print(torch.cuda.get_device_name(0))"
    ])
    print(result.stdout)
    result.check()  # raises if the remote command failed

For DDP-style distributed training that needs file uploads and inter-peer
network access, use :func:`partagpu.distribute` (which is built on top of
``run_remote``).
"""

from __future__ import annotations

import base64
import json
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Mapping, Sequence, Union

import requests

from partagpu.discover import API_BASE, GPUResource


class RemoteTaskError(RuntimeError):
    """Raised when a remote task fails to be dispatched or returns non-zero."""


@dataclass
class TaskResult:
    """Result of a command executed on a remote peer."""

    id: str
    target_machine: str
    status: str
    exit_code: int | None
    stdout: str
    stderr: str

    @property
    def ok(self) -> bool:
        return self.status == "Completed" and (self.exit_code or 0) == 0

    def check(self) -> "TaskResult":
        """Raise RemoteTaskError if the task didn't complete successfully."""
        if not self.ok:
            raise RemoteTaskError(
                f"Tâche distante échouée sur {self.target_machine!r} "
                f"(status={self.status}, exit_code={self.exit_code}). "
                f"stderr:\n{self.stderr}"
            )
        return self

    def __repr__(self) -> str:
        head = (self.stdout or self.stderr).splitlines()[:1]
        preview = head[0][:60] if head else ""
        return (
            f"TaskResult(target={self.target_machine!r}, "
            f"status={self.status!r}, exit_code={self.exit_code}, "
            f"output={preview!r}{'…' if preview else ''})"
        )


PeerLike = Union[GPUResource, str]
WorkspaceArg = Union[
    Mapping[str, Union[str, bytes]],   # {path: content}
    Iterable[Union[str, Path]],         # [path, path, ...]
]


def _resolve_peer_ip(peer: PeerLike) -> str:
    if isinstance(peer, GPUResource):
        return peer.ip
    if isinstance(peer, str):
        return peer.strip()
    raise TypeError(
        f"peer doit être GPUResource ou IP (str), reçu {type(peer).__name__}"
    )


def _build_workspace(workspace: WorkspaceArg | None) -> list[dict]:
    """Normalize a workspace argument into the wire format expected by the app.

    Accepts either:
      - a mapping ``{relative_path: content}`` — content can be ``str`` or ``bytes``;
      - an iterable of file paths — files are read from disk, the workspace
        path is the basename of each.
    """
    if workspace is None:
        return []

    items: list[tuple[str, bytes]] = []

    if isinstance(workspace, Mapping):
        for path, content in workspace.items():
            if isinstance(content, str):
                content_b = content.encode("utf-8")
            elif isinstance(content, (bytes, bytearray, memoryview)):
                content_b = bytes(content)
            else:
                raise TypeError(
                    f"workspace[{path!r}] doit être str ou bytes, reçu "
                    f"{type(content).__name__}"
                )
            items.append((str(path), content_b))
    else:
        # iterable of paths
        for p in workspace:
            path = Path(p)
            if not path.is_file():
                raise FileNotFoundError(f"fichier workspace introuvable : {p}")
            items.append((path.name, path.read_bytes()))

    return [
        {"path": path, "content_b64": base64.b64encode(data).decode("ascii")}
        for path, data in items
    ]


def run_remote(
    peer: PeerLike,
    args: Sequence[str],
    *,
    timeout: int = 300,
    user: str | None = None,
    network: bool = False,
    workspace: WorkspaceArg | None = None,
    api_base: str = API_BASE,
    local_id: str | None = None,
) -> TaskResult:
    """Run ``args`` on the given peer through the PartaGPU app.

    Args:
        peer: A :class:`GPUResource` (from :func:`partagpu.discover`) or a raw
            IP string. The local app is the dispatcher; you do **not** need
            direct network access to ``peer`` from this Python process.
        args: Command split as a list, e.g. ``["python3", "-c", "print(42)"]``.
            Only commands present in the peer's sandbox allowlist will run.
        timeout: Server-side and client-side wall-clock cap, in seconds.
        user: Optional label for the source user (purely informational).
        network: If True, the peer's sandbox keeps host network access.
            Required for distributed-training rendezvous (NCCL/Gloo). Default
            is False (sandbox is network-isolated).
        workspace: Optional files to push to the peer's ``/workspace`` before
            exec. Accepts either ``{relpath: content}`` or a list of file paths
            (basename used as the workspace path). Total payload capped at
            ~16 MB on the peer side.
        api_base: Override the local app URL (default ``http://127.0.0.1:7654``).

    Returns:
        A :class:`TaskResult`. The call **blocks** until the task is in a
        terminal state (Completed / Failed / Cancelled).

    Raises:
        RemoteTaskError: If the local app can't reach the peer, the peer
            rejects the request, or any transport error occurs.

    Example::

        partagpu.run_remote(
            peer,
            ["python3", "train.py"],
            network=True,
            workspace=["./train.py"],
            timeout=1800,
        )
    """
    if not args:
        raise ValueError("args ne peut pas être vide.")

    peer_ip = _resolve_peer_ip(peer)
    if not peer_ip:
        raise ValueError("L'IP du pair est requise.")

    # Pre-allocate the local task id so we can cancel mid-flight on KeyboardInterrupt.
    if local_id is None:
        local_id = str(uuid.uuid4())

    payload = {
        "peer_ip": peer_ip,
        "args": list(args),
        "timeout_secs": int(timeout),
        "network": bool(network),
        "workspace": _build_workspace(workspace),
        "local_id": local_id,
    }
    if user is not None:
        payload["user"] = user

    url = f"{api_base.rstrip('/')}/api/dispatch"
    try:
        resp = requests.post(url, json=payload, timeout=timeout + 60)
    except KeyboardInterrupt:
        # User hit Ctrl+C while we were waiting for the peer. Best effort: tell
        # the local app to send DELETE to the peer so the remote task stops too.
        _try_cancel(api_base, local_id)
        raise
    except requests.ConnectionError as e:
        raise RemoteTaskError(
            "Impossible de joindre l'application PartaGPU sur "
            f"{api_base}. Lancez l'app puis ré-essayez."
        ) from e
    except requests.RequestException as e:
        raise RemoteTaskError(f"Erreur HTTP locale : {e}") from e

    if resp.status_code >= 400:
        try:
            err = resp.json().get("error") or resp.text
        except json.JSONDecodeError:
            err = resp.text
        raise RemoteTaskError(
            f"Dispatch refusé (HTTP {resp.status_code}) : {err}"
        )

    data = resp.json()
    return TaskResult(
        id=data.get("id", ""),
        target_machine=data.get("target_machine", peer_ip),
        status=data.get("status", "Unknown"),
        exit_code=data.get("exit_code"),
        stdout=data.get("output", ""),
        stderr=data.get("error_output", ""),
    )


def _try_cancel(api_base: str, local_id: str, timeout: float = 5.0) -> None:
    """Best-effort POST /api/cancel. Errors are swallowed — used in cleanup paths."""
    try:
        requests.post(
            f"{api_base.rstrip('/')}/api/cancel",
            json={"local_id": local_id},
            timeout=timeout,
        )
    except Exception:  # noqa: BLE001 — cleanup, never fail
        pass


def cancel(local_id: str, *, api_base: str = API_BASE) -> bool:
    """Cancel an in-flight outgoing task by its local id.

    Returns True if the peer acknowledged the cancellation, False if only the
    local state was updated (peer unreachable). Raises RemoteTaskError on
    transport errors.
    """
    try:
        resp = requests.post(
            f"{api_base.rstrip('/')}/api/cancel",
            json={"local_id": local_id},
            timeout=15,
        )
    except requests.RequestException as e:
        raise RemoteTaskError(f"Cancel failed: {e}") from e
    if resp.status_code >= 400 and resp.status_code != 502:
        try:
            err = resp.json().get("error") or resp.text
        except json.JSONDecodeError:
            err = resp.text
        raise RemoteTaskError(f"Cancel refusé (HTTP {resp.status_code}) : {err}")
    return bool(resp.json().get("remote", False))
