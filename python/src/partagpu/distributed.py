"""Distributed training orchestrator for PartaGPU.

The high-level entry point is :func:`distribute`. It takes a Python script
and a list of GPUs (defaults to all GPUs in the room) and runs the script
as a PyTorch DDP job — one process per GPU, all coordinating via NCCL/Gloo
on the LAN.

What it does for you:

- Discovers GPUs via :func:`partagpu.discover` if you don't pass them.
- Picks rank 0 (the first GPU's host) as the master / rendezvous endpoint.
- Sets ``MASTER_ADDR``, ``MASTER_PORT``, ``RANK``, ``WORLD_SIZE``,
  ``LOCAL_RANK`` env vars on each worker.
- Pushes your script (and any extra files) to each peer's sandbox workspace.
- Lifts the sandbox network isolation on each peer (``network=True``) so the
  rendezvous socket can be reached.
- Dispatches all workers in parallel and returns once all of them are in a
  terminal state.

Inside your training script, just initialize DDP the standard way::

    import os
    import torch.distributed as dist

    dist.init_process_group(
        backend=os.environ.get("BACKEND", "nccl"),
        init_method="env://",
    )
    rank = int(os.environ["RANK"])
    world_size = int(os.environ["WORLD_SIZE"])
    # ... train ...
    dist.destroy_process_group()
"""

from __future__ import annotations

import concurrent.futures
import os
import socket
import threading
import uuid
from pathlib import Path
from typing import Sequence

from partagpu.discover import API_BASE, GPUResource, discover
from partagpu.remote import RemoteTaskError, TaskResult, _try_cancel, run_remote


DEFAULT_MASTER_PORT = 29500


def setup_ddp(
    rank: int,
    world_size: int,
    master_addr: str = "127.0.0.1",
    master_port: int = DEFAULT_MASTER_PORT,
    backend: str = "nccl",
) -> None:
    """Initialize a PyTorch DDP process group from the current process.

    This is the manual building block used inside the training script when
    you don't want to use :func:`distribute`. ``RANK``/``WORLD_SIZE``/
    ``MASTER_ADDR``/``MASTER_PORT`` are set in the environment first so
    PyTorch sees them.
    """
    import torch.distributed as dist

    os.environ["MASTER_ADDR"] = master_addr
    os.environ["MASTER_PORT"] = str(master_port)
    os.environ["RANK"] = str(rank)
    os.environ["WORLD_SIZE"] = str(world_size)

    dist.init_process_group(backend=backend, rank=rank, world_size=world_size)


def cleanup_ddp() -> None:
    """Destroy the PyTorch distributed process group, if any."""
    import torch.distributed as dist

    if dist.is_initialized():
        dist.destroy_process_group()


def _local_lan_ip() -> str:
    """Best-effort LAN IP of this machine (the one peers can reach)."""
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.connect(("8.8.8.8", 80))
        ip = s.getsockname()[0]
        s.close()
        return ip
    except OSError:
        return "127.0.0.1"


def _build_workspace_files(paths: Sequence[Path]) -> dict[str, bytes]:
    """Read files from disk into a {basename: bytes} mapping ready to be passed
    as `workspace=` to :func:`run_remote`. The conversion to wire format
    (base64-encoded JSON payload) happens inside `run_remote`."""
    out: dict[str, bytes] = {}
    for p in paths:
        path = Path(p).expanduser().resolve()
        if not path.is_file():
            raise FileNotFoundError(f"fichier introuvable : {p}")
        name = path.name
        if name in out:
            raise ValueError(f"deux fichiers de workspace ont le même nom : {name}")
        out[name] = path.read_bytes()
    return out


def _local_rank_map(gpus: Sequence[GPUResource]) -> list[int]:
    """For each entry in ``gpus``, return its index among entries sharing the
    same host (by IP). Used to assign LOCAL_RANK when several workers run on
    the same machine."""
    counts: dict[str, int] = {}
    out: list[int] = []
    for g in gpus:
        local = counts.get(g.ip, 0)
        out.append(local)
        counts[g.ip] = local + 1
    return out


def distribute(
    script: str | Path,
    args: Sequence[str] = (),
    *,
    gpus: list[GPUResource] | None = None,
    extra_files: Sequence[str | Path] = (),
    master_port: int = DEFAULT_MASTER_PORT,
    backend: str = "nccl",
    timeout: int = 3600,
    user: str | None = None,
    api_base: str = API_BASE,
    live: bool = False,
) -> list[TaskResult]:
    """Run ``script`` as a DDP training across ``gpus``.

    One process per GPU, env-based rendezvous. The local app brokers the
    transport; you don't manage SSH or any low-level launch.

    Args:
        script: Python file to run. Pushed to each peer's sandbox workspace.
        args: Extra CLI args appended to ``python3 <script>``.
        gpus: GPUs to use. Defaults to :func:`partagpu.discover()` (all GPUs
            in the room).
        extra_files: Additional files to ship into each peer's workspace
            alongside the script.
        master_port: TCP port for the rendezvous (must be in 29500-29510 to
            be reachable through the host firewall configured by PartaGPU).
        backend: ``"nccl"`` (GPU) or ``"gloo"`` (CPU/GPU). Default NCCL.
        timeout: Per-worker wall-clock cap, in seconds.
        user: Optional label propagated to every peer's incoming-task panel.
        api_base: Override the local app URL.
        live: If True, print stdout/stderr from each rank as it arrives,
            prefixed with ``[rankN] `` so concurrent output remains readable.
            The full text is still returned in each :class:`TaskResult`.

    Returns:
        A list of :class:`partagpu.TaskResult`, one per rank, in rank order.
        Each ``.stdout`` / ``.stderr`` / ``.exit_code`` reflects what that
        worker produced.

    Raises:
        RuntimeError: If no GPUs are available.
        RemoteTaskError: If the dispatch is refused outright (network/auth).

    Example::

        results = partagpu.distribute(
            "train_ddp.py",
            args=["--epochs", "10"],
            timeout=1800,
        )
        for r in results:
            print(r.target_machine, r.exit_code)
            print(r.stdout[-500:])
    """
    if backend not in ("nccl", "gloo"):
        raise ValueError(f"backend doit être 'nccl' ou 'gloo', reçu {backend!r}")

    if gpus is None:
        gpus = discover(api_base=api_base)
    if not gpus:
        raise RuntimeError(
            "Aucun GPU disponible. Verifiez que PartaGPU tourne, que vous etes "
            "dans une salle, et qu'au moins un pair partage ses ressources."
        )

    world_size = len(gpus)
    script_path = Path(script).expanduser().resolve()
    if not script_path.is_file():
        raise FileNotFoundError(f"script introuvable : {script}")

    files = [script_path] + [Path(p) for p in extra_files]
    workspace = _build_workspace_files(files)
    script_name = script_path.name

    # Master address: rank 0's machine. If rank 0 is "local" with a loopback
    # IP, replace by the actual LAN IP so other ranks can connect.
    master_addr = gpus[0].ip
    if master_addr in ("127.0.0.1", "0.0.0.0", ""):
        master_addr = _local_lan_ip()

    # Effective user label for the incoming-task panel on each peer.
    label = user or os.environ.get("USER", "partagpu")

    # Position of each worker among workers on the SAME host (LOCAL_RANK).
    local_ranks = _local_rank_map(gpus)

    # Pre-allocate one local task id per rank, so we can cancel siblings on
    # failure without waiting for run_remote to return.
    local_ids = [str(uuid.uuid4()) for _ in gpus]

    # Verrou partage entre les rangs en mode live : evite que deux rangs
    # printent une demi-ligne chacun en meme temps. Une seule ligne de stdout
    # est ecrite atomiquement.
    print_lock = threading.Lock() if live else None

    def _launch(rank: int, gpu: GPUResource) -> TaskResult:
        env_prefix = [
            "env",
            f"MASTER_ADDR={master_addr}",
            f"MASTER_PORT={master_port}",
            f"RANK={rank}",
            f"WORLD_SIZE={world_size}",
            "LOCAL_RANK=0",
            f"PARTAGPU_LOCAL_RANK={local_ranks[rank]}",
            f"CUDA_VISIBLE_DEVICES={gpu.device_index}",
            f"BACKEND={backend}",
        ]
        cmd = [*env_prefix, "python3", script_name, *args]
        return run_remote(
            gpu,
            cmd,
            timeout=timeout,
            user=f"{label} (rank {rank}/{world_size}, dev {gpu.device_index})",
            network=True,
            workspace=workspace,
            api_base=api_base,
            local_id=local_ids[rank],
            live=live,
            live_prefix=f"[rank{rank}] " if live else "",
            live_lock=print_lock,
        )

    def _cancel_siblings(except_rank: int, results: list) -> None:
        """Cancel all ranks that haven't produced a result yet. Best effort."""
        for j, lid in enumerate(local_ids):
            if j == except_rank or results[j] is not None:
                continue
            _try_cancel(api_base, lid)

    # Run all workers concurrently. They all need to be alive at roughly the
    # same time for NCCL's rendezvous to converge.
    results: list[TaskResult | Exception | None] = [None] * world_size
    errored_rank: int | None = None
    interrupted = False

    with concurrent.futures.ThreadPoolExecutor(max_workers=world_size) as ex:
        futures = {ex.submit(_launch, i, g): i for i, g in enumerate(gpus)}
        try:
            for fut in concurrent.futures.as_completed(futures):
                i = futures[fut]
                try:
                    r = fut.result()
                    results[i] = r
                    # If this rank failed and we haven't started cancelling
                    # siblings yet, do it now (the others are likely waiting
                    # for an NCCL rendezvous that won't complete).
                    if not r.ok and errored_rank is None:
                        errored_rank = i
                        _cancel_siblings(i, results)
                except Exception as e:  # noqa: BLE001
                    results[i] = e
                    if errored_rank is None:
                        errored_rank = i
                        _cancel_siblings(i, results)
        except KeyboardInterrupt:
            interrupted = True
            for lid in local_ids:
                _try_cancel(api_base, lid)
            # Let workers settle into Cancelled state before re-raising.
            for fut in futures:
                try:
                    fut.result(timeout=10)
                except Exception:  # noqa: BLE001
                    pass
            raise

    if interrupted:
        # Defensive: should already be re-raised above.
        raise KeyboardInterrupt()

    # Surface the first transport-level exception if any (a non-zero exit_code
    # is reported via TaskResult.ok, not raised — the caller can inspect each
    # result individually).
    for i, r in enumerate(results):
        if isinstance(r, Exception):
            raise RemoteTaskError(
                f"Rank {i} a échoué avant de produire un résultat : {r}"
            ) from r

    return results  # type: ignore[return-value]
