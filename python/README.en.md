🇫🇷 [Version française](README.md)

<p align="center">
  <img src="https://raw.githubusercontent.com/cesar-lizurey/partagpu/main/public/favicon.png" alt="PartaGPU" width="320">
</p>

# partagpu

Python client for [PartaGPU](https://github.com/cesar-lizurey/partagpu) — use the GPUs of multiple classroom machines for distributed training.

## Installation

```bash
pip install partagpu
```

## Prerequisites

The PartaGPU app must be running on your machine *and* on every peer, all in the **same room** (same access code), with **sharing enabled** on the target peers.

For DDP: `torch` must be installed in the **system Python** (`/usr/bin/python3`) of every peer — a user venv is not enough (the sandbox runs as the `partagpu` user and cannot see your `$HOME`). On Ubuntu:

```bash
sudo pip install --break-system-packages torch
```

## Discovering GPUs

```python
import partagpu

gpus = partagpu.discover()
# One entry per CUDA device. A PC with 2 GPUs produces 2 entries with
# the same `host` / `ip` but distinct `device_index`.
# [GPU('local',  ip='192.168.70.103', dev=0, limit=100%, verified),
#  GPU('local',  ip='192.168.70.103', dev=1, limit=100%, verified),
#  GPU('César 2', ip='192.168.70.105', dev=0, limit=50%,  verified)]
```

## Running a command on a peer (`run_remote`)

```python
import partagpu

peer = next(g for g in partagpu.discover() if g.host != "local")

result = partagpu.run_remote(
    peer,
    ["python3", "-c", "import torch; print(torch.cuda.get_device_name(0))"],
    timeout=30,
)
print(result.stdout)
result.check()  # raises if exit != 0
```

`run_remote` also accepts:
- `network=True` — let the sandbox keep network access (data downloads, DDP, etc.)
- `workspace={path: content}` or `workspace=[Path, ...]` — push files into the sandbox `/workspace` before exec
- `timeout=int` — seconds (default 300)
- `user="alice"` — informational label shown on the peer
- `local_id="..."` — pre-allocated id so you can cancel the task before it returns
- `live=True` — print the peer's stdout/stderr as it streams (~250 ms cadence) instead of returning everything at the end. Handy for following long-running training runs from a notebook.
- `outputs=["model.pt", ...]` — files to fetch back from the peer's `/workspace` after exit. Available as `result.artifacts: dict[str, bytes]`. Aggregate cap of 256 MiB per task.

`Ctrl+C` in the notebook automatically propagates a cancel to the peer.

## Cancelling a task

```python
# Programmatically, from another notebook or cell
partagpu.cancel(local_id)
```

The `local_id` comes from `TaskResult.id` (returned by `run_remote`/`distribute`) or from a `local_id=` kwarg you set yourself.

## Distributed training (`distribute`)

```python
import partagpu

results = partagpu.distribute(
    "train.py",
    args=["--epochs", "10"],
    extra_files=["config.yaml", "utils.py"],
    timeout=1800,
    live=True,                    # stream logs while running
    outputs=["model.pt"],         # fetch the checkpoint back into RAM
    local=False,                  # skip the local machine
)
for r in results:
    print(r.target_machine, r.exit_code)
    print(r.stdout[-500:])

# The checkpoint is in results[0].artifacts (rank 0 by DDP convention)
import io, torch
weights = torch.load(io.BytesIO(results[0].artifacts["model.pt"]), map_location="cpu")
```

`distribute`:
- discovers every GPU in the room (unless you pass `gpus=`). **Multi-GPU per machine** is handled: a PC with 4 GPUs contributes 4 workers.
- pushes `train.py` (and `extra_files`) into the sandbox of every peer.
- sets `MASTER_ADDR`, `MASTER_PORT`, `RANK`, `WORLD_SIZE`, `LOCAL_RANK`, `CUDA_VISIBLE_DEVICES`, `PARTAGPU_LOCAL_RANK`, `BACKEND` on every worker.
- pins each worker to a single physical GPU via `CUDA_VISIBLE_DEVICES` (the script always uses `cuda:0`, regardless of the physical index).
- opens the sandbox network isolation on every peer (NCCL/Gloo rendezvous).
- launches `world_size` workers in parallel (on their respective machines) and waits for every result.

Extra optional parameters:
- `live=True` — each rank prints its logs prefixed with `[rankN]` as they arrive (a shared lock keeps lines from interleaving between ranks).
- `outputs=["model.pt", ...]` — paths (relative to `/workspace`) to fetch back from each rank after exit. By DDP convention only rank 0 saves a checkpoint, so only `results[0].artifacts` is typically non-empty. Aggregate cap of 256 MiB per task.
- `local=False` — exclude the local machine from auto-discovery. Useful when sharing isn't enabled locally and you just want to consume remote GPUs. Without this filter, rank 0 lands on the local peer-API and gets a 403.

In `train.py`, initialize DDP the standard way:

```python
import os
import torch.distributed as dist

dist.init_process_group(backend=os.environ["BACKEND"], init_method="env://")
rank = int(os.environ["RANK"])
world_size = int(os.environ["WORLD_SIZE"])
# ... your training loop ...
dist.destroy_process_group()
```

## Manual backend setup (`setup_ddp` / `cleanup_ddp`)

If you'd rather orchestrate DDP yourself (without `distribute`), minimal helpers are available:

```python
from partagpu.distributed import setup_ddp, cleanup_ddp

setup_ddp(rank=0, world_size=2, master_addr="192.168.70.103", backend="nccl")
# ... your DDP code ...
cleanup_ddp()
```

## See also

- [Main README](https://github.com/cesar-lizurey/partagpu) — app install, room management, UI
- [Architecture](https://github.com/cesar-lizurey/partagpu/blob/main/docs/ARCHITECTURE.en.md) — how peer-to-peer dispatch, the sandbox, and DDP work
- [Troubleshooting](https://github.com/cesar-lizurey/partagpu/blob/main/docs/TROUBLESHOOTING.en.md) — common errors (HMAC auth mismatch, NCCL, sandbox, missing torch, etc.)
- Example notebook: [examples/decouverte_gpu.ipynb](https://github.com/cesar-lizurey/partagpu/blob/main/examples/decouverte_gpu.ipynb)
- Smoke tests: [smoke_run_remote.py](https://github.com/cesar-lizurey/partagpu/blob/main/examples/smoke_run_remote.py), [smoke_ddp.py](https://github.com/cesar-lizurey/partagpu/blob/main/examples/smoke_ddp.py), [smoke_multi_gpu.py](https://github.com/cesar-lizurey/partagpu/blob/main/examples/smoke_multi_gpu.py)
