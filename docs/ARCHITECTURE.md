# Architecture de PartaGPU

Ce document explique **comment PartaGPU fonctionne en interne** : les composants, les protocoles, la sécurité, et comment l'orchestration DDP se branche sur l'infrastructure pair-à-pair. Pour le guide utilisateur, voir le [README principal](../README.md) et le [README du package Python](../python/README.md).

---

## Table des matières

1. [Vue d'ensemble](#vue-densemble)
2. [Les deux serveurs HTTP](#les-deux-serveurs-http)
3. [Authentification TOTP entre pairs](#authentification-totp-entre-pairs)
4. [Le sandbox d'exécution](#le-sandbox-dexécution)
5. [Flux d'une tâche `run_remote`](#flux-dune-tâche-run_remote)
6. [Orchestration DDP avec `distribute`](#orchestration-ddp-avec-distribute)
7. [Multi-GPU par machine](#multi-gpu-par-machine)
8. [Découverte mDNS](#découverte-mdns)
9. [Privilèges et helper](#privilèges-et-helper)
10. [Modèle de sécurité](#modèle-de-sécurité)

---

## Vue d'ensemble

PartaGPU est une application Tauri (backend Rust + frontend React) qui transforme un PC en **nœud de calcul partageable** sur un LAN. Une fois plusieurs PC dans la même *salle* (même secret TOTP), ils forment un cluster ad-hoc capable d'exécuter du code arbitraire — typiquement du PyTorch DDP — réparti sur tous les GPU disponibles.

### Composants

```
┌──────────────────────── Machine A (mon poste) ────────────────────────┐
│                                                                       │
│   ┌────────────┐     ┌─────────────────┐     ┌──────────────────┐   │
│   │ Notebook   │     │  App PartaGPU   │     │   peer A (sandbox │   │
│   │ Python     │ ──> │   (Tauri + UI)  │ ──> │   bubblewrap)     │   │
│   └────────────┘     │                 │     └──────────────────┘   │
│                      │  http_api 7654  │                             │
│                      │  peer_api 7655  │ <───── peers d'autres machines
│                      │  mDNS browse    │                             │
│                      └─────────────────┘                             │
└───────────────────────────────────────────────────────────────────────┘
                                ▲
                                │ peer-to-peer over LAN (TOTP-signed)
                                ▼
┌──────────────────────── Machine B (autre poste) ──────────────────────┐
│   App PartaGPU + sandbox bubblewrap exécutant les tâches de A         │
└───────────────────────────────────────────────────────────────────────┘
```

- **Frontend** (React + TypeScript, Vite) : 3 onglets *Mon partage* / *Mon utilisation* / *Guide*. Communique avec le backend via Tauri `invoke`.
- **Backend Rust** (`src-tauri/src/`) : modules pour auth, discovery, sandbox, sharing, monitoring, deux serveurs HTTP, journal de sécurité.
- **Helper Rust privilégié** (`src-tauri/helper/`, binaire séparé) : opérations qui demandent root (création d'utilisateur, cgroups, firewall). Lancé via `pkexec` avec règle PolicyKit dédiée.
- **Package Python** (`python/src/partagpu/`) : client minimal (`requests` only) qui parle à l'API locale `127.0.0.1:7654` pour découvrir les GPU et dispatcher des tâches.

---

## Les deux serveurs HTTP

L'app expose **deux** serveurs HTTP, écrits à la main en Rust avec `tokio` (pas de framework, ~150 LOC chacun). C'est délibéré : ces APIs ont des audiences et des règles d'auth très différentes.

### `127.0.0.1:7654` — API locale

**Audience** : les clients Python sur la même machine, et le frontend Tauri (lecture seule).

**Auth** : aucune (binding sur loopback uniquement, donc seuls les processus locaux y accèdent).

**Routes** :
- `GET /api/peers` — liste des pairs découverts via mDNS (sérialisation de `Vec<Peer>`)
- `GET /api/gpu` — liste des GPU disponibles, **une entrée par device CUDA** (champ `device_index`). Pour le local : énumération via `nvidia-smi`. Pour les pairs vérifiés qui partagent : expansion selon `peer.gpu_count`.
- `GET /api/status` — état local du partage (Active/Paused/Disabled + limites)
- `POST /api/dispatch` — **soumission d'une tâche à un pair**. Body :
  ```json
  {
    "peer_ip": "192.168.70.105",
    "args": ["python3", "-c", "..."],
    "timeout_secs": 60,
    "network": true,
    "workspace": [{"path": "train.py", "content_b64": "..."}]
  }
  ```
  Le handler :
  1. Récupère le code TOTP courant de l'AuthManager local
  2. Crée une entrée `OutgoingTasks` avec status `Queued` (UI visible immédiatement)
  3. Sur un thread `spawn_blocking` : POST vers `<peer_ip>:7655/peer/v1/tasks` avec header `X-PartaGPU-TOTP`
  4. Récupère `task_id`, met l'OutgoingTask en `Running`
  5. Poll `<peer_ip>:7655/peer/v1/tasks/<task_id>` toutes les 500 ms
  6. Quand la tâche atteint un état terminal (`Completed`/`Failed`), met à jour OutgoingTasks et **retourne le `Task` complet** au client
  7. Si timeout : marque comme failed et retourne 502.

  Cet endpoint est **bloquant** par design : le notebook Python attend simplement le résultat. Pour des tâches longues, on pourrait passer en async (Phase 4 du roadmap).

### `0.0.0.0:7655` — API pair-à-pair

**Audience** : les autres machines PartaGPU sur le LAN.

**Auth** : header `X-PartaGPU-TOTP` validé par `AuthManager::verify_code`. Le code est valide si :
- L'app locale est `is_joined()` (dans une salle)
- Le partage est `Active`
- Le code passé matche `current_code() ± 1 step` (TOTP, fenêtre ±30 s pour tolérer le clock skew)

**Routes** :
- `GET /peer/v1/health` — pas d'auth, retourne `{hostname, version, in_room, sharing_active}`. Sert de probe.
- `POST /peer/v1/tasks` — **réception d'une tâche** d'un pair vérifié. Body :
  ```json
  {
    "args": [...],
    "source_user": "alice",
    "timeout_secs": 60,
    "network_enabled": true,
    "workspace": [...]
  }
  ```
  Auth → résolution du `source_machine` depuis l'IP TCP source (lookup dans `discovery.get_peers()`) → `IncomingTasks::create_and_run(...)` qui lance le sandbox dans un thread, retourne immédiatement un `task_id`.
- `GET /peer/v1/tasks/<id>` — auth idem, retourne la struct `Task` complète (status, output, error_output, exit_code).

Pourquoi un serveur séparé du 7654 ? Parce que 7654 est **loopback** (sécurité : pas exposé au réseau) tandis que 7655 doit être joignable depuis le LAN. Mélanger les deux dans un même serveur compliquerait l'auth (lecture seule sans auth d'un côté, écriture avec auth TOTP de l'autre).

---

## Authentification TOTP entre pairs

Le système est **partagé-secret** : tous les membres d'une salle dérivent le même secret TOTP à partir de la passphrase de 4 mots.

### Flux de création de salle

1. L'app génère un secret aléatoire de 20 bytes
2. Les **4 premiers bytes** indexent dans un `WORDLIST` de 256 mots français → la passphrase (4 mots, ~4 milliards de combinaisons)
3. La passphrase est dictée à l'oral aux camarades
4. Au join, l'app reconvertit la passphrase en bytes, puis pad avec `SHA1(seed)[..16]` pour reconstruire les 20 bytes du secret canonique
5. Le secret est sauvegardé en `~/.config/partagpu/room.json`

### Vérification mutuelle

Chaque pair **annonce son code TOTP courant** dans son TXT record mDNS (champ `totp`). Les autres pairs comparent à leur propre `current_code()` :

- Si match (à ±1 step près, soit ±30 s pour tolérer la dérive d'horloge) → `verified=true`
- Sinon → `verified=false`, peer affiché grisé dans l'UI

### Auth des requêtes HTTP entre pairs

Pour `POST /peer/v1/tasks` et `GET /peer/v1/tasks/<id>`, le client envoie son code TOTP courant dans l'en-tête `X-PartaGPU-TOTP: 123456`. Le récepteur vérifie avec `AuthManager::verify_code` (même logique que mDNS, fenêtre ±1 step).

**Pourquoi pas TLS ?** Le LAN après auth de salle est de confiance (modèle "salle de cours"). Un attaquant qui écoute le trafic verrait passer du Python source mais ne pourrait pas injecter ses propres tâches sans le secret. Pour une protection contre l'écoute, le chiffrement AES dérivé du secret de salle est dans `TODO.md` (Phase suivante).

---

## Le sandbox d'exécution

Toute tâche reçue d'un pair tourne dans un sandbox **bubblewrap** (`bwrap`), exécuté via `IncomingTasks::create_and_run` → `Sandbox::execute_with_options` (cf. [src-tauri/src/sandbox.rs](../src-tauri/src/sandbox.rs)).

### Flags bwrap appliqués

```
bwrap \
  --ro-bind /usr /usr  --ro-bind /lib /lib  --ro-bind /lib64 /lib64 \
  --ro-bind /bin /bin  --ro-bind /sbin /sbin  --ro-bind /etc /etc \
  --proc /proc \
  --dev /dev \
  --dev-bind /dev/nvidia0 /dev/nvidia0 \         # GPU passthrough
  --dev-bind /dev/nvidiactl /dev/nvidiactl \     # (boucle pour /dev/nvidia*)
  --dev-bind /dev/nvidia-uvm /dev/nvidia-uvm \
  --bind /tmp/partagpu-task-<uuid> /workspace \  # workspace host-bind
  --chdir /workspace \
  --tmpfs /tmp \
  [--unshare-net]                                # SI network_enabled=false
  --unshare-pid \
  --die-with-parent \
  --new-session \
  --uid <partagpu> --gid <partagpu> \
  -- <args>
```

### Caractéristiques de sécurité

- **Système de fichiers en lecture seule** sauf `/workspace` et `/tmp` (tmpfs)
- **Pas de network par défaut** (`--unshare-net`). Levée seulement si `network_enabled=true` dans la requête (requis pour DDP).
- **PID namespace isolé** : la tâche ne peut pas voir / signaler les processus de l'hôte
- **Run as `partagpu` UID** : compte dédié sans accès aux home directories des autres
- **Cgroup partagpu** : limite CPU/RAM imposée par les sliders de l'UI
- **Pas de `$HOME` utilisateur** : le sandbox ne voit rien de votre home

### Workspace : transfert de fichiers

Le client (Python ou un autre pair) peut envoyer des fichiers à matérialiser dans `/workspace` avant exec. Implémenté ainsi :

1. Le client encode chaque fichier en base64 et envoie `[{path, content_b64}, ...]` dans le body POST
2. Le serveur valide chaque path (relatif, pas de `..`, pas de NUL)
3. Crée un dir temporaire sur l'hôte (`/tmp/partagpu-task-<uuid>`, mode 0777 pour que l'UID partagpu puisse écrire)
4. Décode et écrit chaque fichier (mode 0666)
5. bwrap fait `--bind <tempdir> /workspace`
6. Quand la tâche se termine, le tempdir est supprimé (Drop sur `TempWorkspace`)

Limite globale : **16 Mo** par tâche (configurable via `MAX_WORKSPACE_BYTES`). Pour les datasets plus gros : pre-installation côté pair, ou Phase 4 (file streaming).

### GPU passthrough

Le sandbox bind tous les `/dev/nvidia*` détectés à l'exec. CUDA + NCCL fonctionnent normalement à l'intérieur, à condition que les libs userspace (`libcuda.so`, `libcudart.so`, etc., `libnvidia-ml.so`) soient sous `/usr/lib` (qui est r/o-bound).

### Allowlist

Seules les commandes dans `Sandbox::allowlist` peuvent être lancées. Defaults : `python3`, `python`, `nvidia-smi`, `bash`, `make`, `gcc`, `julia`, `Rscript`, etc. Géré depuis l'UI (page *Mon partage* → onglet *Allowlist*) ou via les Tauri commands `add_to_allowlist` / `remove_from_allowlist`.

---

## Flux d'une tâche `run_remote`

```
┌─ NOTEBOOK ─────────────────────────┐
│ partagpu.run_remote(peer, args,    │
│                     network=…,     │
│                     workspace=…)   │
└────────────┬───────────────────────┘
             │ POST http://127.0.0.1:7654/api/dispatch
             ▼
┌─ APP LOCALE (machine A) ───────────┐
│ http_api::handle_dispatch          │
│  1. auth.current_code() → "123456" │
│  2. OutgoingTasks::add(Queued)     │
│  3. spawn_blocking task            │
└────────────┬───────────────────────┘
             │ POST http://<peer_ip>:7655/peer/v1/tasks
             │ X-PartaGPU-TOTP: 123456
             │ body: {args, source_user, timeout_secs,
             │        network_enabled, workspace}
             ▼
┌─ APP DISTANTE (machine B) ─────────┐
│ peer_api::handle_submit            │
│  1. check_auth (TOTP, in_room,     │
│     sharing active)                │
│  2. resolve source_machine from IP │
│  3. log "task accepted"            │
│  4. IncomingTasks::create_and_run  │
│     → spawn thread → Sandbox::exec │
│        (bwrap, GPU passthrough,    │
│         /workspace tmpfs, etc.)    │
│  5. retourne {task_id, accepted}   │
└────────────┬───────────────────────┘
             │ 200 OK + task_id
             │
             │ ◄── machine A poll : GET /peer/v1/tasks/<id>
             │     toutes les 500 ms
             │
             │ ◄── tâche en cours : status=Running
             │
             │ ◄── tâche terminée : status=Completed,
             │     stdout/stderr/exit_code remplis
             ▼
┌─ APP LOCALE ───────────────────────┐
│ Met à jour OutgoingTasks (Completed│
│ + output/error/exit_code)          │
│ Retourne Task complet au notebook  │
└────────────┬───────────────────────┘
             │ JSON Task
             ▼
┌─ NOTEBOOK ─────────────────────────┐
│ TaskResult avec stdout, stderr, …  │
└────────────────────────────────────┘
```

Pendant l'exécution, **l'UI de la machine A** affiche 1 outgoing task (page *Mon utilisation*), et **l'UI de la machine B** affiche 1 incoming task (page *Mon partage*).

---

## Orchestration DDP avec `distribute`

`partagpu.distribute(script, args=, ...)` ([python/src/partagpu/distributed.py](../python/src/partagpu/distributed.py)) bâtit sur `run_remote` :

1. **Découverte** : `partagpu.discover()` → `world_size = len(gpus)`
2. **Master address** : IP du rang 0. Si "local" avec IP loopback, remplace par l'IP LAN (`_local_lan_ip()`)
3. **Workspace** : lecture du `script` + `extra_files` → `dict[str, bytes]` (basename → contenu)
4. **Pour chaque rang `i`** :
   - `LOCAL_RANK_OF_HOST = position-among-host-workers` (calculé par `_local_rank_map(gpus)`)
   - `CUDA_VISIBLE_DEVICES = gpu.device_index` (filtre à un seul GPU physique)
   - `LOCAL_RANK = 0` (cohérent avec le filtre CVD)
   - `PARTAGPU_LOCAL_RANK = position-on-host` (informatif, pour les logs)
   - Cmd : `["env", "MASTER_ADDR=…", …, "python3", script_name, *args]`
5. **Lancement parallèle** : `ThreadPoolExecutor(max_workers=world_size)` qui submit `run_remote(...)` pour chaque rang. Chaque appel est bloquant individuellement, mais ils tournent tous en concurrent.
6. **Attente** : `as_completed` collecte les `TaskResult`, range par RANK, retourne `list[TaskResult]`.

### Rendezvous NCCL

Dans le sandbox de chaque pair :
- `--unshare-net` est **omis** (parce que `network_enabled=true`)
- Donc le sandbox utilise le namespace réseau de l'hôte
- Le rang 0 binde `0.0.0.0:MASTER_PORT` (ouvert dans le firewall via le helper)
- Les autres rangs se connectent en TCP à `MASTER_ADDR:MASTER_PORT`
- NCCL fait son `init_process_group`, l'all-reduce démarre

Le port par défaut est **29500** (range ouverte par le helper : 29500–29510). Pour lancer plusieurs entraînements concurrents, passez `master_port=29501` etc.

---

## Multi-GPU par machine

Une machine qui a N GPU physiques contribue **N entrées distinctes** à `discover()` : même `host`/`ip`, `device_index` différent (0 à N-1).

### Annonce

`Discovery::register` ajoute `gpu_count` aux propriétés mDNS. Calculé via `crate::resource::list_gpus().len()` qui parse `nvidia-smi --query-gpu=index,name,...`. Cache : aucun (re-query à chaque refresh, ~50 ms).

### Variable de simulation

Pour tester la logique multi-GPU sans matériel adéquat : `PARTAGPU_FORCE_GPU_COUNT=4 npm run tauri:dev`. La fonction `list_gpus()` génère alors 4 `GpuDevice` synthétiques. Utile pour vérifier que `distribute()` génère les bonnes env vars (cf. `examples/smoke_multi_gpu.py`).

### Dispatch

Côté Python (`distributed.py::distribute`) :
- `_local_rank_map(gpus)` parcourt la liste, compte les workers par IP, donne à chacun sa position-on-host
- Chaque worker reçoit son propre `CUDA_VISIBLE_DEVICES = device_index` → ne voit qu'un seul GPU à l'intérieur
- `LOCAL_RANK = 0` partout (cohérent avec le filtre CVD : un seul GPU visible = index 0)
- Le script utilise `cuda:0`, peu importe le GPU physique réel

### Pourquoi `LOCAL_RANK = 0` et pas la position-on-host ?

Si on filtrait CVD à un seul GPU **et** qu'on mettait `LOCAL_RANK = N`, un script qui fait `torch.cuda.set_device(LOCAL_RANK)` planterait (essaie de set device N alors qu'un seul est visible). En forçant `LOCAL_RANK = 0`, tous les patterns torchrun-compatibles fonctionnent. La vraie position-on-host reste accessible via `PARTAGPU_LOCAL_RANK` pour les besoins de logging.

---

## Découverte mDNS

`mdns-sd` crate. Service type : `_partagpu._tcp.local.`, port 7654 (plus pour la convention que pour la pertinence — l'IP est ce qui compte).

Properties annoncées :
- `hostname` (système)
- `display_name` (nom personnalisé via UI, persisté)
- `sharing` (`true` si Active)
- `cpu_limit`, `ram_limit`, `gpu_limit` (limites des sliders)
- `gpu_count` (nombre de CUDA devices détectés, **nouveau Phase 3**)
- `totp` (code à 6 chiffres courant, change toutes les 30 s)

Le browser (`Discovery::start_browsing`) consomme les events `ServiceResolved` / `ServiceRemoved`, applique :
- **Rate limiting** par pair : 1 update / 2 s (anti-flood)
- **Max peers** : 50 (anti-DOS)
- **Detection de hostname conflict** (deux IPs pour le même hostname → flag `hostname_conflict` + log alerte)
- **Vérification TOTP** → `verified` boolean

Re-announcement périodique (`start_mdns_refresh`) toutes les 5 s **si l'état a changé** (TOTP code, sharing status, limits, gpu_count) — évite le flood quand rien ne bouge.

---

## Privilèges et helper

Les opérations qui demandent root passent par un binaire séparé `partagpu-helper` (workspace member sous `src-tauri/helper/`), invoqué via `pkexec` avec une règle PolicyKit dédiée (`com.partagpu.policy`).

### Sous-commandes du helper

| Cmd | Quand |
|---|---|
| `create-user` | Première activation du partage : crée `partagpu` UID 997, shell `partagpu-shell` (rejette `-c`), bloque SSH/sudo, autostart desktop |
| `set-password` | Définition du mot de passe (lit stdin pour ne pas exposer le mot de passe en CLI args) |
| `setup-cgroup <cpu> <ram>` | Crée/ajuste `/sys/fs/cgroup/partagpu/{cpu.max, memory.max}`. Les ajustements suivants se font en écriture directe (l'UID utilisateur peut écrire dans le cgroup une fois créé). |
| `open-port` | Ouvre TCP 7654, **TCP 7655 (peer)**, **TCP 29500–29510 (DDP)**, UDP 5353 (mDNS) via `ufw` ou `iptables` |
| `close-port` | Ferme les mêmes ports |
| `remove-user` | Supprime complètement `partagpu` + cgroup + règles SSH/sudo |

### Quand pkexec est-il invoqué ?

Seulement pour `create-user`, `set-password`, `setup-cgroup` (premier appel), `open-port`, `close-port`, `remove-user`. Les ajustements de sliders et le monitoring **n'invoquent jamais pkexec** — tout se fait par lecture/écriture directe.

---

## Modèle de sécurité

Voir [SECURITY.md](../SECURITY.md) pour le détail. En résumé :

| Couche | Mécanisme |
|---|---|
| Découverte | mDNS sur LAN. Rate-limit + max peers + détection conflict d'hostnames. |
| Authentification | TOTP partagé (passphrase 4 mots → secret 20 bytes → code 6 chiffres). Vérifié par mDNS + header HTTP. |
| Tâches entrantes | Refusées si pair non vérifié OU sharing pas Active OU TOTP invalide. Logged dans `SecurityLog`. |
| Exécution | bubblewrap : FS r/o, network unshare par défaut, PID unshare, `partagpu` UID, allowlist de commandes |
| Limites | Cgroups v2 (CPU max, memory max). Outputs cappés à 1 Mo stdout / 256 Ko stderr. Workspace cappé à 16 Mo. Timeout configurable. |
| Privilèges | Helper Rust séparé via pkexec, règle PolicyKit explicite. Inputs validés (entiers, longueur, NUL/newline interdits) avant d'atteindre la couche shell. |

### Limites connues

- **Pas de chiffrement** entre pairs sur le LAN (Phase 4 : AES-GCM dérivé du secret de salle)
- **Pas d'isolation par tâche** au niveau cgroup (toutes les tâches partagent le cgroup `partagpu`). Une tâche peut consommer toutes les ressources allouées à `partagpu`.
- **Workspace lit/écrit comme partagpu UID** — deux tâches sur le même pair ont chacune leur dir mais n'ont pas d'isolation forte au-delà de l'UUID du dir.

---

## Pour aller plus loin

- [README principal](../README.md) — vue d'ensemble + guide utilisateur
- [README du package Python](../python/README.md) — référence des APIs Python
- [TROUBLESHOOTING.md](TROUBLESHOOTING.md) — diagnostic des erreurs courantes
- [SECURITY.md](../SECURITY.md) — détail des mesures de sécurité
- [TODO.md](../TODO.md) — ce qui reste à faire
- Code source :
  - Backend Rust : [`src-tauri/src/`](../src-tauri/src/)
  - Helper privilégié : [`src-tauri/helper/src/main.rs`](../src-tauri/helper/src/main.rs)
  - Package Python : [`python/src/partagpu/`](../python/src/partagpu/)
  - Frontend : [`src/`](../src/)
