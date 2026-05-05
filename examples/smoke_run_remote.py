"""End-to-end smoke test for PartaGPU `run_remote` (loopback).

Run this AFTER:
  1. The new app is built and running:  npm run tauri:dev
  2. You've joined or created a room (any name, any passphrase)
  3. Sharing is enabled (toggle "Mon partage")

The script dispatches a tiny Python command to *this same machine* (loopback)
to validate the full path: Python → /api/dispatch → :7655 peer API → sandbox → response.
"""

import sys
import time

import requests

import partagpu

API = "http://127.0.0.1:7654"


def must(cond, msg):
    if not cond:
        print(f"  FAIL  {msg}")
        sys.exit(1)
    print(f"  OK    {msg}")


def main() -> int:
    print("== Pre-checks ==")

    try:
        status = requests.get(f"{API}/api/status", timeout=2).json()
    except requests.RequestException as e:
        print(f"  FAIL  l'app PartaGPU n'est pas joignable sur {API} : {e}")
        return 1
    must(True, f"app reachable, sharing status = {status['status']}")
    must(status["status"] == "Active",
         "le partage doit etre 'Active' (toggle 'Mon partage' dans l'app)")

    try:
        peer_health = requests.get("http://127.0.0.1:7655/peer/v1/health", timeout=2).json()
    except requests.RequestException as e:
        print(f"  FAIL  serveur pair-a-pair injoignable sur :7655 : {e}")
        print(f"        Avez-vous bien rebuild l'app (npm run tauri:dev) ?")
        return 1
    must(True, f"peer API up, version={peer_health.get('version')}, in_room={peer_health.get('in_room')}")
    must(peer_health.get("in_room"), "vous devez etre dans une salle PartaGPU (creer ou rejoindre)")
    must(peer_health.get("sharing_active"), "le partage doit etre actif")

    # Find local IP (the dispatcher will use it to reach the peer API on localhost)
    import socket
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.connect(("8.8.8.8", 80))
    local_ip = s.getsockname()[0]
    s.close()
    print(f"  OK    local IP detected: {local_ip}")

    print("\n== Test 1 : run_remote vers soi-meme (boucle locale) ==")
    t0 = time.time()
    result = partagpu.run_remote(
        local_ip,
        ["python3", "-c", "print('hello from peer'); print(40 + 2)"],
        timeout=30,
    )
    dt = time.time() - t0
    print(f"  {dt:.2f}s  status={result.status}  exit={result.exit_code}")
    print(f"  stdout: {result.stdout.strip()!r}")
    if result.stderr:
        print(f"  stderr: {result.stderr.strip()!r}")
    must(result.ok, "task should complete with exit_code 0")
    must("hello from peer" in result.stdout, "stdout should contain expected text")
    must("42" in result.stdout, "arithmetic should run on the peer")

    print("\n== Test 2 : commande qui echoue (exit 7) ==")
    bad = partagpu.run_remote(
        local_ip,
        ["python3", "-c", "import sys; sys.exit(7)"],
        timeout=15,
    )
    must(bad.exit_code == 7, f"exit_code should be 7, got {bad.exit_code}")
    must(bad.status == "Failed", f"status should be Failed, got {bad.status}")

    print("\n== Test 3 : commande hors allowlist (sandbox refuse) ==")
    blocked = partagpu.run_remote(
        local_ip,
        ["/bin/ls", "/"],
        timeout=10,
    )
    must(blocked.status == "Failed", "non-allowlisted command must fail")
    print(f"  err: {blocked.stderr.strip()[:120]!r}")

    print("\n== Tout est OK. Verifiez dans l'app : ==")
    print("  - 'Mon utilisation'    : 3 taches sortantes (la cible est vous-meme)")
    print("  - 'Mon partage'        : 3 taches entrantes (vous etes aussi le pair)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
