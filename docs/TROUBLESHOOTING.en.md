🇫🇷 [Version française](TROUBLESHOOTING.md)

# Troubleshooting — what to do when it breaks

Common errors hit while using the app **and** the `partagpu` Python package, with their cause and fix. For usage basics, see the [README](../README.en.md). For technical details, see [ARCHITECTURE.en.md](ARCHITECTURE.en.md).

---

## Application side

### The app doesn't start / crashes on launch

```bash
# Run from a console to see the logs
/usr/bin/partagpu          # installed version
# or
npm run tauri:dev          # dev version
```

Typical causes:
- **No Wayland/X session**: required for the Tauri GUI.
- **Missing Tauri/webkit** packages: `sudo apt install libwebkit2gtk-4.1-dev`.

### Sharing fails to enable

The UI prompts for the password (pkexec) and then errors out. Causes:
- **PolicyKit not installed**: `sudo apt install policykit-1`.
- **Helper not installed**: re-run `sudo bash scripts/install-helper.sh`.
- **Wrong password**: it's the **administrator** password of the machine that's asked, not the password of the `partagpu` account.

### Banner "Failed to initialize NVML: Driver/Library version mismatch"

The loaded NVIDIA kernel module is on a different version than the userland libs (typically after an `apt upgrade` without a reboot).

```bash
# Check
cat /proc/driver/nvidia/version           # module version
ls -l /usr/lib/x86_64-linux-gnu/libnvidia-ml.so*    # libs version
dpkg -l | grep nvidia-driver              # package version

# Fix: reboot
sudo reboot
```

---

## Peers and discovery

### A peer doesn't show up in the list

Check, in order:
1. **Both machines have the app running** (`ps -ef | grep partagpu`).
2. **The peer is on the same subnet** (same first three IP octets in general). PartaGPU has no NAT traversal.
3. **The firewall allows UDP 5353 (mDNS)**:
   ```bash
   sudo ufw status | grep 5353
   ```
4. **Avahi is running** (system mDNS daemon, some setups need it): `sudo systemctl status avahi-daemon`.

### Peer shows up but marked "unverified"

The active HMAC challenge on `/peer/v1/verify` didn't return a matching response. Causes:
- **Not in the same room**: different passphrases → different `auth_key` → verify HMAC mismatch. *Leave the room* on one side and rejoin with the right code.
- **Inconsistent PartaGPU versions**: every peer in a room must run the same major version of PartaGPU. Check the version badge in the app header on each machine.
- **Probe timeout (3 s)**: firewall blocking port 7655, peer far on the LAN, or the peer's app is still starting up. Re-verification runs every 60 s, wait a minute.

Clock skew doesn't affect `/verify` (nonce-based, not time-windowed), but it does block dispatches (HTTP `X-PartaGPU-AUTH` is ±30 s anti-replay). Enabling NTP everywhere is still recommended:
```bash
sudo timedatectl set-ntp true
timedatectl status      # check System clock synchronized: yes
```

### Multiple peers with the same hostname (badge "Conflict")

Two machines announce the same `uname -n`. No risk to functionality (PartaGPU disambiguates by IP), but the UI shows a warning. To silence it:
```bash
sudo hostnamectl set-hostname pc-room-104    # new hostname
sudo reboot
```

---

## Local HTTP API

### `partagpu.discover()` returns 0 GPUs

```bash
# 1. App reachable?
curl -s http://127.0.0.1:7654/api/status

# 2. Does mDNS see peers?
curl -s http://127.0.0.1:7654/api/peers | python3 -m json.tool

# 3. Does the app see the local GPU?
curl -s http://127.0.0.1:7654/api/gpu | python3 -m json.tool
```

Depending on what's missing:
- `/api/status` doesn't answer → the app didn't start its HTTP server. Crash on startup? Check the logs (`npm run tauri:dev`).
- `/api/peers` is empty → no peer discovered. See *Peers and discovery* above.
- `/api/gpu` shows no local GPU → `nvidia-smi` is broken (driver/lib mismatch — see above).
- Peers are listed but none appears in `/api/gpu` → they're not sharing (`sharing_enabled=false`) or not verified (`verified=false`). Ask them.

---

## `run_remote` and `distribute`

### `RemoteTaskError: Dispatch refused (HTTP 412): ... PartaGPU room`

You're not in any room. UI → top tab → *Create a room* or *Join a room*.

### `RemoteTaskError: Peer ... refused the task (HTTP 401): invalid auth`

Either the room mismatches (wrong `auth_key`), or clock skew between the two PCs exceeds 30 s, or the header was tampered with in transit. See *Peers and discovery* ; the security log details the exact cause.

### `HTTP 415 Unsupported Media Type` from the peer

Every peer-to-peer body is encrypted (AES-256-GCM). The receiving peer returns 415 when:
- The client sends a plaintext body → check PartaGPU versions on both machines.
- The Content-Type is not `application/x-partagpu-encrypted-v1`.
- The peer is in a different room (the HMAC header would normally fail first with 401, since the `auth_key` is derived from the same secret as the `room_key`).

To verify both peers share the same secret: on each machine, *My sharing* → *Room* should show the same name and passphrase.

### `Command refused: "X" is not in the allowed list`

The peer's allowlist doesn't contain that command. On the peer's machine: UI → *My sharing* → *Allowlist* → add the binary. Defaults: `python3`, `python`, `nvidia-smi`, `bash`, `make`, `gcc`, `julia`, etc.

### Sandbox crashes with `Permission denied (os error 13)`

The peer's PartaGPU is out of date — `git pull && npm run tauri:dev` on the peer to update. The workspace must live under `/tmp` (writable by the app), not under `/var/lib/partagpu` (mode 700, owned by `partagpu`).

### A task is stuck in `Queued` indefinitely

The sandbox can't start. Causes:
- **`bubblewrap` not installed on the target machine**: `sudo apt install -y bubblewrap`.
- **The `partagpu` account wasn't created**: on the target, UI → *My sharing* → *Disable* then *Enable* (re-runs the user creation through the helper).

### `ModuleNotFoundError: No module named 'torch'` on the peer

The sandbox runs as the `partagpu` UID, which can't see your user venv. Two solutions:

**Recommended — through the UI (managed venv)**: on every target machine, *My sharing* → *Python environment for incoming tasks* → click **Install ML toolkit (~3 GB)**. Admin password asked once, 5–10 min download. Installs `torch`, `torchvision`, `numpy`, `scipy`, `pandas`, `scikit-learn`, `matplotlib`, `pillow`. The sandbox then bind-mounts `/var/lib/partagpu/venv/` automatically and aliases `python3` to it. No system Python pollution.

**Alternative — system install**:
```bash
sudo apt install -y python3-pip
sudo /usr/bin/python3 -m pip install --break-system-packages \
  torch torchvision numpy scipy pandas scikit-learn matplotlib pillow
```

Do this on **every machine** that should accept PyTorch tasks.

**To add a package to the managed venv** (e.g. transformers, jax):

```bash
sudo /var/lib/partagpu/venv/bin/pip install transformers
```

(No dedicated UI for this yet; install directly with the venv's pip. Requires *Install ML toolkit* to have been clicked first.)

### `Failed to initialize NumPy: No module named 'numpy'` (torch startup warning)

Benign (torch still works) but annoying. Fix: install numpy in system Python as above.

### NCCL hangs at `init_process_group`

Port 29500 (rendezvous) isn't reachable between machines. Test from machine A:
```bash
nc -zv 192.168.x.y 29500     # IP of machine B
```

If refused/timeout:
- **Firewall not open on the target**. Check: `sudo ufw status | grep 29500`. If empty: toggle sharing off then on (re-runs the helper which opens the port). Or directly: `sudo ufw allow 29500:29510/tcp`.
- **Helper out of date** on the target. `git pull && npm run helper:build && sudo bash scripts/install-helper.sh && npm run tauri:dev`.

### `CUDA error: invalid device ordinal`

The script uses `cuda:1` (or higher) while `CUDA_VISIBLE_DEVICES` already pins it to a single GPU. Always use `cuda:0` or `cuda:LOCAL_RANK` (which equals 0) inside a script launched by `partagpu.distribute`.

### `distribute()` raises `RemoteTaskError: Rank N failed before producing a result`

A Python exception was raised on **your** side before the task even reached the peer. Read the full message — typically:
- A missing Python dependency on **your** machine (not the peer).
- An `extra_files` path that doesn't exist.
- A mistyped argument to `partagpu.distribute(...)`.

### Truncated outputs

stdout is capped at **1 MB**, stderr at **256 KB** per task. For larger outputs, write to a file (and consider exfiltrating it through a shared filesystem or a separate upload — not handled by PartaGPU yet).

### My `print()` only show up at the end (not live)

Python block-buffers stdout when it's not a TTY (our case: pipe to the sandbox). Three ways to force line-by-line flushing:

```python
print("hello", flush=True)            # explicit at every call
```

```bash
python3 -u my_script.py                # global unbuffered mode
```

```python
import os
os.environ.setdefault("PYTHONUNBUFFERED", "1")  # at the top of the script
```

Classic symptom: the live panel stays empty for 30 s, then everything floods at the end. Almost always Python buffering, not an infra bug.

`tqdm` and `print('\\r…', end='')` (progress bars) write without newlines and are buffered differently — `tqdm(file=sys.stderr)` often helps.

---

## Smoke tests

To verify the install at every stage:

```bash
cd examples
source venv/bin/activate     # or use ./venv/bin/python directly
```

| Script | What it validates | When to run it |
|---|---|---|
| `smoke_run_remote.py` | Sandboxed command dispatch, loopback | After first setup, to check app + sandbox |
| `smoke_ddp.py` | DDP loopback world_size=1, then multi-machine if `PARTAGPU_TEST_MULTI=1` | Before launching a real DDP training |
| `smoke_multi_gpu.py` | Multi-GPU dispatch logic (with `PARTAGPU_FORCE_GPU_COUNT=2` when launching the app) | To verify dispatch on multi-GPU servers |

If one fails, the cause is usually listed in one of the sections above.

---

## Cancelling a running task

Three ways, in order of usage:

1. **Stop button in the UI**: each row of *My running tasks* (outgoing) or *Who is using my resources?* (incoming) shows a **Stop** button while the task is `Queued` or `Running`. Click → immediate cancel, propagated to the peer (SIGTERM in the sandbox, SIGKILL after 2 s if no response).

2. **`Ctrl+C` in a notebook**: `partagpu.run_remote(...)` and `partagpu.distribute(...)` intercept `KeyboardInterrupt`, send a `POST /api/cancel` to the local app (which forwards as `DELETE` to the peer), then re-raise. For `distribute`, **all** ranks are cancelled.

3. **Programmatically**: `partagpu.cancel(local_id)` where `local_id` is the `id` returned in `TaskResult`. Useful to cancel from another notebook or script.

```python
import partagpu, threading

def long_task():
    partagpu.run_remote(peer, ["python3", "-c", "import time; time.sleep(3600)"],
                        timeout=3600, local_id="my-test-task")

t = threading.Thread(target=long_task)
t.start()

# Later, from another cell or piece of code:
partagpu.cancel("my-test-task")
```

### `distribute()`: why do my ranks linger when one crashes?

If a rank dies mid-DDP, the others would in theory stay blocked on `init_process_group` or an `all-reduce` until the NCCL timeout (~30 min). `distribute()` detects the first rank that fails and **automatically cancels** all the others, so no machine is left waiting in the void.

### A task is marked `Cancelled` in the UI but seems to keep running

SIGTERM can be ignored by some scripts (Python signal handler that doesn't lead to a hard kill). The 2 s timer then sends `SIGKILL`. If after 5 s the task still appears "Running":
- Check the helper is up to date (`npm run helper:build && sudo bash scripts/install-helper.sh`).
- Peer-side logs: see the `npm run tauri:dev` terminal. If you spot `kill: not found` or similar, the helper doesn't have the `kill` tool on PATH — fix the `partagpu` account's PATH or install `procps`.

---

## Logs and observability

### App logs

```bash
# Dev mode: everything is in the npm run tauri:dev terminal
npm run tauri:dev 2>&1 | tee /tmp/partagpu.log

# Production mode: no dedicated log file, run from a terminal
/usr/bin/partagpu
```

### Security log (built into the UI)

Tab *My sharing* → *Security log*. Keeps the last 500 events (peer detected, tasks accepted/refused, hostname conflicts, etc.).

### Inspect the state of a running task

```bash
# List incoming tasks (peer side)
curl -s -H "X-PartaGPU-AUTH: 123456" \
    http://127.0.0.1:7655/peer/v1/tasks/<task-id> | python3 -m json.tool
```

(The header must be computed via `compute_request_auth(method, path, body)` ; calling this from `curl` is awkward — prefer a small Python helper that computes the HMAC.)

---

## When nothing else works

Full reset to a clean state:

```bash
# 1. Leave the room in the UI (clears ~/.config/partagpu/room.json)
# 2. Disable sharing in the UI

# 3. Stop the app
pkill -f /usr/bin/partagpu
pkill -f target/debug/partagpu

# 4. Remove the partagpu user (back to initial state)
sudo /usr/local/lib/partagpu/partagpu-helper remove-user

# 5. Clear the config
rm -rf ~/.config/partagpu

# 6. Relaunch the app, redo the config (create room, enable sharing, etc.)
npm run tauri:dev
```

Use this when you've lost trust in the system state after a string of failed experiments. User-level reset only, doesn't touch the system install.
