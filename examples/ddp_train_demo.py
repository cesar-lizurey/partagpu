"""Minimal DDP training script — exercises PartaGPU's `distribute()` end-to-end.

This script is designed to be shipped to each peer's sandbox via
`partagpu.distribute(...)`. It reads the standard DDP env vars (RANK,
WORLD_SIZE, MASTER_ADDR, MASTER_PORT, plus BACKEND set by PartaGPU) and
runs a tiny CNN over synthetic data with NCCL/Gloo all-reduce between ranks.

Usage (don't run by hand — use partagpu.distribute):

    import partagpu
    results = partagpu.distribute("ddp_train_demo.py", args=["--epochs", "3"])

The script is intentionally self-contained: no external data files, no
relative imports. Every peer that runs it needs `torch` available to its
system Python.
"""

import argparse
import os
import socket
import sys
import time

import torch
import torch.distributed as dist
import torch.nn as nn
from torch.nn.parallel import DistributedDataParallel as DDP
from torch.utils.data import DataLoader, DistributedSampler, TensorDataset


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser()
    p.add_argument("--epochs", type=int, default=3)
    p.add_argument("--batch-size", type=int, default=32)
    p.add_argument("--samples", type=int, default=1024)
    return p.parse_args()


def pick_backend() -> str:
    """Use NCCL on CUDA hosts, fall back to Gloo (works CPU-only)."""
    env_backend = os.environ.get("BACKEND", "").lower()
    if env_backend in ("nccl", "gloo"):
        # Honor explicit choice if it can actually run.
        if env_backend == "nccl" and not torch.cuda.is_available():
            return "gloo"
        return env_backend
    return "nccl" if torch.cuda.is_available() else "gloo"


def main() -> int:
    args = parse_args()
    rank = int(os.environ["RANK"])
    world_size = int(os.environ["WORLD_SIZE"])
    master_addr = os.environ.get("MASTER_ADDR", "?")
    master_port = os.environ.get("MASTER_PORT", "?")
    backend = pick_backend()

    host = socket.gethostname()
    print(f"[rank {rank}/{world_size}] host={host} backend={backend}", flush=True)
    print(
        f"[rank {rank}] cuda={torch.cuda.is_available()} "
        f"master={master_addr}:{master_port}",
        flush=True,
    )

    dist.init_process_group(backend=backend, init_method="env://")
    print(f"[rank {rank}] DDP initialized", flush=True)

    device = torch.device("cuda:0" if torch.cuda.is_available() else "cpu")
    if device.type == "cuda":
        print(f"[rank {rank}] gpu={torch.cuda.get_device_name(0)}", flush=True)

    # Synthetic dataset (3x32x32 images, 10 classes) — small so this runs
    # quickly enough to verify the wiring.
    torch.manual_seed(rank)
    X = torch.randn(args.samples, 3, 32, 32)
    y = torch.randint(0, 10, (args.samples,))
    ds = TensorDataset(X, y)
    sampler = DistributedSampler(ds, num_replicas=world_size, rank=rank, shuffle=True)
    loader = DataLoader(ds, batch_size=args.batch_size, sampler=sampler)

    model = nn.Sequential(
        nn.Conv2d(3, 16, 3, padding=1),
        nn.ReLU(),
        nn.Conv2d(16, 32, 3, padding=1),
        nn.ReLU(),
        nn.AdaptiveAvgPool2d(4),
        nn.Flatten(),
        nn.Linear(32 * 4 * 4, 10),
    ).to(device)

    if device.type == "cuda":
        model = DDP(model, device_ids=[0])
    else:
        model = DDP(model)

    opt = torch.optim.AdamW(model.parameters(), lr=1e-3)
    crit = nn.CrossEntropyLoss()

    for epoch in range(args.epochs):
        sampler.set_epoch(epoch)
        running, n = 0.0, 0
        t0 = time.time()
        for bx, by in loader:
            bx = bx.to(device, non_blocking=True)
            by = by.to(device, non_blocking=True)
            opt.zero_grad(set_to_none=True)
            loss = crit(model(bx), by)
            loss.backward()
            opt.step()
            running += loss.item()
            n += 1
        # Every rank prints to make the parallelism visible in the output.
        print(
            f"[rank {rank}] epoch {epoch + 1}/{args.epochs} "
            f"loss={running / max(n, 1):.4f} time={time.time() - t0:.2f}s",
            flush=True,
        )

    if rank == 0:
        print("[rank 0] training complete", flush=True)
    dist.destroy_process_group()
    return 0


if __name__ == "__main__":
    sys.exit(main())
