"""Script de test pour l'UI dispatcher de PartaGPU.

A pousser via "Ajouter..." dans le panneau "Lancer une commande sur un pair",
puis lancer avec la commande : python3 test_dispatch.py

Verifie :
- L'upload du workspace (le fichier est bien arrive cote pair)
- L'execution sandboxee (hostname, cwd, user dans le sandbox)
- Le streaming des logs (5 lignes a 1s d'intervalle)
- L'access GPU si torch est installe (system Python ou venv gere)
"""

import os
import socket
import sys
import time


def main() -> None:
    bar = "=" * 60
    print(bar, flush=True)
    print(f"hostname : {socket.gethostname()}", flush=True)
    print(f"user     : {os.environ.get('USER', '?')}", flush=True)
    print(f"pid      : {os.getpid()}", flush=True)
    print(f"cwd      : {os.getcwd()}", flush=True)
    print(f"python   : {sys.executable}", flush=True)
    print(f"version  : {sys.version.split()[0]}", flush=True)
    print(bar, flush=True)

    # --- (1) Verification du workspace ---
    print("\n[1/3] Fichiers presents dans /workspace :", flush=True)
    files = sorted(os.listdir("."))
    for f in files:
        try:
            size = os.path.getsize(f)
            print(f"  - {f} ({size} octets)", flush=True)
        except OSError as e:
            print(f"  - {f} (erreur stat : {e})", flush=True)
    if "test_dispatch.py" in files:
        print("  -> Le fichier uploade est bien arrive.", flush=True)
    else:
        print("  -> ATTENTION : le fichier upload n'apparait pas !", flush=True)

    # --- (2) Streaming des logs ---
    print("\n[2/3] Test streaming (5 ticks, 1s d'intervalle) :", flush=True)
    print("       Vous devez voir les lignes arriver une a une dans l'UI.", flush=True)
    for i in range(5):
        print(f"  tick {i + 1}/5 a {time.strftime('%H:%M:%S')}", flush=True)
        time.sleep(1)

    # --- (3) GPU access via torch ---
    print("\n[3/3] Test torch + GPU (optionnel) :", flush=True)
    try:
        import torch  # noqa: PLC0415

        print(f"  torch          : {torch.__version__}", flush=True)
        print(f"  cuda available : {torch.cuda.is_available()}", flush=True)
        if torch.cuda.is_available():
            n = torch.cuda.device_count()
            print(f"  device count   : {n}", flush=True)
            for i in range(n):
                p = torch.cuda.get_device_properties(i)
                vram_gb = p.total_memory / 1e9
                print(f"    GPU {i}: {p.name}  ({vram_gb:.1f} Go VRAM)", flush=True)
            # Petit matmul pour verifier l'acces compute
            a = torch.randn(2000, 2000, device="cuda")
            b = torch.randn(2000, 2000, device="cuda")
            c = (a @ b).sum().item()
            print(f"  matmul 2000x2000 sur GPU : sum = {c:,.1f}", flush=True)
        else:
            print("  CUDA pas disponible cote sandbox.", flush=True)
            print("  -> Verifier que /dev/nvidia* sont bien bindes,", flush=True)
            print("     ou que le driver hote est actif (nvidia-smi marche).", flush=True)
    except ImportError:
        print("  torch n'est pas installe cote pair :", flush=True)
        print("    - Soit installer le venv gere : Mon partage -> Environnement Python", flush=True)
        print("    - Soit en system Python : sudo /usr/bin/python3 -m pip install --break-system-packages torch", flush=True)

    print("\nFini.", flush=True)


if __name__ == "__main__":
    main()
