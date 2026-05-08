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
import sys
import threading
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import IO, Callable, Iterable, Mapping, Optional, Sequence, Union

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


class _LivePoller:
    """Background thread that polls `GET /api/tasks/<id>/output` and prints
    incoming stdout/stderr chunks. Used by `run_remote(live=True)` so the
    notebook can show live training logs while `/api/dispatch` blocks on
    its main connection.

    Lines are split client-side and prefixed with `prefix` (typically
    "[rank0] " when called from `distribute(live=True)`) so concurrent
    ranks remain readable.
    """

    POLL_INTERVAL_SEC = 0.25
    REQUEST_TIMEOUT = 5.0

    def __init__(
        self,
        api_base: str,
        local_id: str,
        prefix: str,
        stdout: IO[str],
        stderr: IO[str],
        lock: Optional[threading.Lock],
    ) -> None:
        self.api_base = api_base.rstrip("/")
        self.local_id = local_id
        self.prefix = prefix
        self.stdout = stdout
        self.stderr = stderr
        self.lock = lock or threading.Lock()
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._stdout_offset = 0
        self._stderr_offset = 0
        self._stdout_tail = ""
        self._stderr_tail = ""

    def start(self) -> None:
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        # Final poll pour rattraper les derniers octets ecrits entre le dernier
        # tick et la terminaison de la tache distante.
        self._poll_once()
        self._flush_tails()
        # Le thread se termine seul au prochain tick ; on attend un court
        # delai pour eviter de laisser un thread daemon en vol.
        self._thread.join(timeout=self.POLL_INTERVAL_SEC * 2)

    def _run(self) -> None:
        while not self._stop.is_set():
            self._poll_once()
            self._stop.wait(self.POLL_INTERVAL_SEC)

    def _poll_once(self) -> None:
        url = (
            f"{self.api_base}/api/tasks/{self.local_id}/output"
            f"?stdout_since={self._stdout_offset}&stderr_since={self._stderr_offset}"
        )
        try:
            resp = requests.get(url, timeout=self.REQUEST_TIMEOUT)
        except Exception:  # noqa: BLE001 — best-effort, ne pas casser le user code
            return
        if resp.status_code != 200:
            return
        try:
            data = resp.json()
        except json.JSONDecodeError:
            return
        out_chunk = data.get("stdout_chunk") or ""
        err_chunk = data.get("stderr_chunk") or ""
        if out_chunk:
            self._stdout_offset = data.get("stdout_total", self._stdout_offset)
            self._emit(self.stdout, "_stdout_tail", out_chunk)
        if err_chunk:
            self._stderr_offset = data.get("stderr_total", self._stderr_offset)
            self._emit(self.stderr, "_stderr_tail", err_chunk)

    def _emit(self, stream: IO[str], tail_attr: str, chunk: str) -> None:
        # Concatener avec le reliquat du tour precedent (un chunk peut couper
        # une ligne en plein milieu) puis split sur '\n' pour prefixer chaque
        # ligne completer ; on garde le reliquat (apres le dernier \n) pour
        # le prochain tick.
        buf = getattr(self, tail_attr) + chunk
        lines = buf.split("\n")
        complete_lines, remainder = lines[:-1], lines[-1]
        setattr(self, tail_attr, remainder)
        if not complete_lines:
            return
        with self.lock:
            for line in complete_lines:
                stream.write(f"{self.prefix}{line}\n")
            stream.flush()

    def _flush_tails(self) -> None:
        # Si la tache se termine sans \n final, on print quand meme le
        # dernier morceau pour ne pas perdre de sortie.
        with self.lock:
            if self._stdout_tail:
                self.stdout.write(f"{self.prefix}{self._stdout_tail}\n")
                self.stdout.flush()
                self._stdout_tail = ""
            if self._stderr_tail:
                self.stderr.write(f"{self.prefix}{self._stderr_tail}\n")
                self.stderr.flush()
                self._stderr_tail = ""


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
    live: bool = False,
    live_prefix: str = "",
    live_stdout: Optional[IO[str]] = None,
    live_stderr: Optional[IO[str]] = None,
    live_lock: Optional[threading.Lock] = None,
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
        live: If True, poll the local app every ~250 ms and print stdout/stderr
            chunks as they arrive (instead of returning everything at the end).
            ``live_prefix`` is prepended to each line ; useful when several
            ranks of :func:`distribute` print concurrently.
        live_prefix: String prepended to each printed line in live mode.
        live_stdout / live_stderr: Files to print to in live mode (default
            ``sys.stdout`` / ``sys.stderr``).
        live_lock: Optional :class:`threading.Lock` shared with other live
            calls so concurrent ranks don't garble each other's lines.

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

    # Live mode : on lance le POST /api/dispatch dans un thread (qui reste
    # bloque jusqu'a la fin) pendant qu'on poll /api/tasks/<id>/output sur la
    # connexion principale et qu'on print les chunks au fur et a mesure.
    poller: Optional[_LivePoller] = None
    if live:
        poller = _LivePoller(
            api_base=api_base,
            local_id=local_id,
            prefix=live_prefix,
            stdout=live_stdout or sys.stdout,
            stderr=live_stderr or sys.stderr,
            lock=live_lock,
        )
        poller.start()

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
    finally:
        if poller is not None:
            poller.stop()

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
