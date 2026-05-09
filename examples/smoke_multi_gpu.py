"""Smoke test for multi-GPU-per-host dispatch logic.

Validates that ``partagpu.distribute()`` generates the right env vars for
each worker when a single host advertises N GPUs:

  - global RANK                = 0..world_size-1
  - WORLD_SIZE                 = total devices across all peers
  - LOCAL_RANK                 = 0 for every worker (CUDA_VISIBLE_DEVICES isolates)
  - PARTAGPU_LOCAL_RANK        = position among workers on the same host
  - CUDA_VISIBLE_DEVICES       = the assigned physical device index

Run this WITHOUT real multi-GPU hardware by starting the app with
``PARTAGPU_FORCE_GPU_COUNT=2`` set on the **machine running the app**:

    PARTAGPU_FORCE_GPU_COUNT=2 npm run tauri:dev

Then in another terminal:

    ./venv/bin/python smoke_multi_gpu.py

The probe script doesn't import torch, so it tests only the dispatch logic
(workspace upload, env var threading). NCCL itself can only be exercised on
a host with real multiple GPUs.
"""

import sys
from pathlib import Path

import partagpu


def step(msg: str) -> None:
    print(f"\n== {msg} ==")


def must(cond: bool, label: str) -> None:
    if cond:
        print(f"  OK    {label}")
    else:
        print(f"  FAIL  {label}")
        sys.exit(1)


def main() -> int:
    step("Découverte")
    gpus = partagpu.discover()
    print(f"        {len(gpus)} entrée(s) GPU :")
    for g in gpus:
        print(f"          {g}")

    locals_only = [g for g in gpus if g.host == "local"]
    must(
        len(locals_only) >= 2,
        f"au moins 2 GPU 'local' attendus (vu : {len(locals_only)})."
        f"\n        Avez-vous lancé l'app avec PARTAGPU_FORCE_GPU_COUNT=2 ?"
    )

    step("Dispatch via distribute() avec un probe sans torch")
    here = Path(__file__).parent
    probe = here / "_probe_env.py"
    probe.write_text(
        "import os, socket\n"
        "for k in ('RANK', 'WORLD_SIZE', 'LOCAL_RANK', 'PARTAGPU_LOCAL_RANK',\n"
        "          'MASTER_ADDR', 'MASTER_PORT', 'CUDA_VISIBLE_DEVICES'):\n"
        "    print(f'{k}={os.environ.get(k, \"<missing>\")}', flush=True)\n"
        "print(f'host={socket.gethostname()}', flush=True)\n"
    )

    try:
        results = partagpu.distribute(
            probe,
            gpus=locals_only,
            timeout=30,
            backend="gloo",  # avoid NCCL (no real multi-GPU here)
        )
    finally:
        try:
            probe.unlink()
        except OSError:
            pass

    must(len(results) == len(locals_only),
         f"{len(locals_only)} résultats attendus, eu {len(results)}")

    expected_world = len(locals_only)
    for rank, r in enumerate(results):
        print(f"\n  rank {rank} ({r.target_machine}, exit {r.exit_code}):")
        for line in r.stdout.splitlines():
            print(f"    | {line}")
        if r.stderr.strip():
            print("    stderr:")
            for line in r.stderr.splitlines()[:3]:
                print(f"    | {line}")

        env = dict(
            line.split("=", 1)
            for line in r.stdout.splitlines()
            if "=" in line and not line.startswith("host=")
        )
        must(env.get("RANK") == str(rank), f"rank {rank} : RANK={env.get('RANK')}")
        must(env.get("WORLD_SIZE") == str(expected_world),
             f"rank {rank} : WORLD_SIZE={env.get('WORLD_SIZE')}")
        must(env.get("LOCAL_RANK") == "0",
             f"rank {rank} : LOCAL_RANK doit être 0 (CVD filtre)")
        must(env.get("PARTAGPU_LOCAL_RANK") == str(rank),
             f"rank {rank} : PARTAGPU_LOCAL_RANK={env.get('PARTAGPU_LOCAL_RANK')}")
        must(env.get("CUDA_VISIBLE_DEVICES") == str(rank),
             f"rank {rank} : CUDA_VISIBLE_DEVICES={env.get('CUDA_VISIBLE_DEVICES')}")

    print("\nMulti-GPU dispatch logic OK.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
