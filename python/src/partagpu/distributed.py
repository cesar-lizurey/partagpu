"""Helpers for distributed PyTorch training across PartaGPU peers.

Usage in a Jupyter notebook:

    import partagpu
    gpus = partagpu.discover()
    # → [GPU('local', ip='192.168.70.103', limit=100%, verified),
    #    GPU('César 2', ip='192.168.70.105', limit=50%, verified)]

    from partagpu.distributed import setup_ddp, cleanup_ddp

    # On each node, call setup_ddp with the appropriate rank
    setup_ddp(rank=0, world_size=len(gpus), master_addr=gpus[0].ip)
    ...
    cleanup_ddp()

For single-machine multi-GPU or simple remote offloading, use the
higher-level `distribute` context manager.
"""

from __future__ import annotations

import os
import subprocess
import sys
from contextlib import contextmanager
from typing import TYPE_CHECKING

from partagpu.discover import GPUResource, discover

if TYPE_CHECKING:
    pass


def setup_ddp(
    rank: int,
    world_size: int,
    master_addr: str = "127.0.0.1",
    master_port: int = 29500,
    backend: str = "nccl",
) -> None:
    """Initialize a PyTorch Distributed Data Parallel process group.

    Args:
        rank: Global rank of this process.
        world_size: Total number of processes (= number of GPUs).
        master_addr: IP of the rank-0 node.
        master_port: Port for the rendezvous.
        backend: Communication backend ('nccl' for GPU, 'gloo' for CPU).
    """
    import torch.distributed as dist

    os.environ["MASTER_ADDR"] = master_addr
    os.environ["MASTER_PORT"] = str(master_port)
    os.environ["RANK"] = str(rank)
    os.environ["WORLD_SIZE"] = str(world_size)

    dist.init_process_group(backend=backend, rank=rank, world_size=world_size)


def cleanup_ddp() -> None:
    """Destroy the PyTorch distributed process group."""
    import torch.distributed as dist

    if dist.is_initialized():
        dist.destroy_process_group()


def launch_workers(
    script: str,
    gpus: list[GPUResource] | None = None,
    master_port: int = 29500,
    args: list[str] | None = None,
) -> list[subprocess.Popen]:
    """Launch distributed training workers on available GPUs.

    This starts one subprocess per GPU. The local GPU gets rank 0,
    remote GPUs get subsequent ranks. Each worker receives RANK,
    WORLD_SIZE, MASTER_ADDR, and MASTER_PORT as environment variables.

    Args:
        script: Path to the training script.
        gpus: List of GPUResource (defaults to partagpu.discover()).
        master_port: Port for the rendezvous server.
        args: Additional arguments to pass to the training script.

    Returns:
        List of Popen objects for each worker.
    """
    if gpus is None:
        gpus = discover()

    if not gpus:
        raise RuntimeError("Aucun GPU disponible. Verifiez PartaGPU.")

    master_addr = gpus[0].ip
    world_size = len(gpus)
    workers = []

    for rank, gpu in enumerate(gpus):
        env = os.environ.copy()
        env["MASTER_ADDR"] = master_addr
        env["MASTER_PORT"] = str(master_port)
        env["RANK"] = str(rank)
        env["WORLD_SIZE"] = str(world_size)
        env["LOCAL_RANK"] = "0"  # one GPU per process

        cmd = [sys.executable, script] + (args or [])

        if gpu.host == "local" or gpu.ip == master_addr:
            # Local worker
            proc = subprocess.Popen(cmd, env=env)
        else:
            # Remote worker via SSH to the partagpu account
            remote_cmd = " ".join(
                [f"{k}={v}" for k, v in env.items()
                 if k in ("MASTER_ADDR", "MASTER_PORT", "RANK", "WORLD_SIZE", "LOCAL_RANK")]
                + cmd
            )
            proc = subprocess.Popen(
                ["ssh", f"partagpu@{gpu.ip}", remote_cmd],
                env=env,
            )

        workers.append(proc)

    return workers


@contextmanager
def distribute(
    gpus: list[GPUResource] | None = None,
    master_port: int = 29500,
    backend: str = "nccl",
):
    """Context manager for distributed training.

    Discovers GPUs, sets up the DDP process group for rank 0, and
    provides the list of GPUs. Cleans up on exit.

    Usage:
        with partagpu.distributed.distribute() as gpus:
            model = DDP(model)
            train(model)
    """
    if gpus is None:
        gpus = discover()

    if not gpus:
        raise RuntimeError("Aucun GPU disponible. Verifiez PartaGPU.")

    master_addr = gpus[0].ip
    world_size = len(gpus)

    setup_ddp(
        rank=0,
        world_size=world_size,
        master_addr=master_addr,
        master_port=master_port,
        backend=backend,
    )

    try:
        yield gpus
    finally:
        cleanup_ddp()
