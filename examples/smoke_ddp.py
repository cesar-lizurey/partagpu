"""End-to-end smoke test for `partagpu.distribute` on a SINGLE machine (loopback).

Run this AFTER:
  1. The new app is built and running:  npm run tauri:dev
  2. Helper reinstalled (port 29500-29510 open):  sudo bash scripts/install-helper.sh
  3. You've created/joined a room and enabled sharing.

What it tests in order, with graceful skips when prerequisites are missing:

  Test 1 — workspace + network wiring (no torch needed)
    Pushes a tiny script that just prints its env vars; runs with network=True.
    Verifies workspace upload + network_enabled flag.

  Test 2 — GPU device passthrough (needs system-wide torch)
    Runs `python3 -c "import torch; print(torch.cuda.is_available())"` inside
    a sandboxed peer. SKIPS if torch is not in the peer's system python.

  Test 3 — distribute() world_size=1 (loopback DDP, needs system torch)
    Real partagpu.distribute(...) call with the demo training script.

  Test 4 — distribute() world_size=2 (multi-machine, needs PARTAGPU_TEST_MULTI=1
    and another sharing peer in the room)

Set PARTAGPU_TEST_MULTI=1 to enable the multi-machine test.
"""

import os
import sys
import time
from pathlib import Path

import partagpu

HERE = Path(__file__).parent
DEMO_SCRIPT = HERE / "ddp_train_demo.py"


def step(msg: str) -> None:
    print(f"\n== {msg} ==")


def must(cond: bool, label: str) -> None:
    if cond:
        print(f"  OK    {label}")
    else:
        print(f"  FAIL  {label}")
        sys.exit(1)


def warn(label: str, detail: str = "") -> None:
    print(f"  SKIP  {label}")
    if detail:
        for line in detail.splitlines():
            print(f"        {line}")


def has_system_torch(local: partagpu.GPUResource) -> bool:
    """Cheap probe: try `import torch` in the peer's sandboxed system python."""
    r = partagpu.run_remote(
        local,
        ["python3", "-c", "import torch; print('ok')"],
        timeout=15,
    )
    return r.ok and "ok" in r.stdout


def main() -> int:
    step("Etat de la salle / partage")
    gpus = partagpu.discover()
    must(len(gpus) >= 1, f"discover() voit au moins 1 GPU (vu : {len(gpus)})")
    print(f"        gpus = {gpus}")
    local = next((g for g in gpus if g.host == "local"), gpus[0])
    must(DEMO_SCRIPT.is_file(), f"trouvé {DEMO_SCRIPT.name}")

    # ── Test 1 ──────────────────────────────────────────────
    step("Test 1 : upload workspace + network_enabled (sans torch)")
    probe = (
        "import os, sys, socket\n"
        "print('host', socket.gethostname())\n"
        "print('rank', os.environ.get('RANK', '?'))\n"
        "print('world', os.environ.get('WORLD_SIZE', '?'))\n"
        "print('master', os.environ.get('MASTER_ADDR', '?'),"
        " ':', os.environ.get('MASTER_PORT', '?'))\n"
        "print('cwd_files', sorted(os.listdir('.')))\n"
    )
    r = partagpu.run_remote(
        local,
        [
            "env",
            "RANK=0",
            "WORLD_SIZE=1",
            "MASTER_ADDR=127.0.0.1",
            "MASTER_PORT=29500",
            "python3", "probe.py",
        ],
        timeout=30,
        network=True,
        workspace={"probe.py": probe},
    )
    print(f"        {r}")
    print(f"        stdout:\n{r.stdout}")
    if r.stderr.strip():
        print(f"        stderr:\n{r.stderr}")
    must(r.ok, "le script poussé via workspace s'est exécuté")
    must("rank 0" in r.stdout, "RANK reçu côté pair")
    must("world 1" in r.stdout, "WORLD_SIZE reçu côté pair")
    must("'probe.py'" in r.stdout, "fichier workspace présent dans cwd")

    # ── Test 2 ──────────────────────────────────────────────
    step("Test 2 : GPU + torch dans le sandbox du pair")
    if not has_system_torch(local):
        warn(
            "torch n'est pas dans le python système du pair",
            "Pour activer DDP sur ce pair :\n"
            "  sudo apt install python3-pip\n"
            "  sudo pip install --break-system-packages torch\n"
            "(les packages installés dans un venv utilisateur ne sont PAS\n"
            "visibles depuis le sandbox, qui tourne en user 'partagpu')",
        )
        print("\nWiring vérifié (Test 1). Tests 3 et 4 ne peuvent pas tourner sans system torch.")
        return 0

    r = partagpu.run_remote(
        local,
        [
            "python3", "-c",
            (
                "import torch;"
                "print('cuda', torch.cuda.is_available());"
                "print('gpu', torch.cuda.get_device_name(0)"
                " if torch.cuda.is_available() else 'no-gpu')"
            ),
        ],
        timeout=30,
    )
    print(f"        {r}")
    print(f"        stdout: {r.stdout!r}")
    must(r.ok, "import torch a fonctionné")
    if "cuda True" in r.stdout:
        print("        OK    le GPU est visible depuis le sandbox du pair")
    else:
        print("        WARN  CUDA = False côté pair. Vérifiez nvidia-smi.")

    # ── Test 3 ──────────────────────────────────────────────
    step("Test 3 : distribute() world_size=1 (loopback DDP)")
    t0 = time.time()
    results = partagpu.distribute(
        DEMO_SCRIPT,
        args=["--epochs", "1", "--samples", "256"],
        gpus=[local],
        master_port=29500,
        timeout=120,
    )
    print(f"        completed in {time.time() - t0:.1f}s")
    must(len(results) == 1, "1 résultat attendu")
    r0 = results[0]
    print(f"        rank 0: status={r0.status} exit={r0.exit_code}")
    print("        stdout:")
    for line in r0.stdout.splitlines():
        print(f"          | {line}")
    if r0.stderr.strip():
        print("        stderr (head):")
        for line in r0.stderr.splitlines()[:8]:
            print(f"          | {line}")
    must(r0.ok, "DDP rank 0 a terminé OK")
    must("DDP initialized" in r0.stdout, "DDP a bien initialisé le process group")
    must("training complete" in r0.stdout, "l'entraînement a fini proprement")

    # ── Test 4 (multi-machine, opt-in) ──────────────────────
    if os.environ.get("PARTAGPU_TEST_MULTI") == "1":
        remote = [g for g in gpus if g.host != "local"]
        must(len(remote) >= 1, "au moins un pair distant pour le test multi")
        chosen = [local, remote[0]]
        step(f"Test 4 : distribute() world_size=2 ({local.host} + {remote[0].host})")
        results = partagpu.distribute(
            DEMO_SCRIPT,
            args=["--epochs", "1", "--samples", "256"],
            gpus=chosen,
            timeout=180,
        )
        for i, r in enumerate(results):
            print(
                f"        rank {i} ({r.target_machine}): "
                f"status={r.status} exit={r.exit_code}"
            )
            for line in r.stdout.splitlines()[-5:]:
                print(f"          | {line}")
        for i, r in enumerate(results):
            must(r.ok, f"rank {i} OK")

    print("\nTout est OK.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
