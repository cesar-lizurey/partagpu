# Architecture de PartaGPU

Ce document explique **comment PartaGPU fonctionne en interne** : les composants, les protocoles, la sécurité, et comment l'orchestration DDP se branche sur l'infrastructure pair-à-pair. Pour le guide utilisateur, voir le [README principal](../README.md) et le [README du package Python](../python/README.md).

---

## Table des matières

1. [Vue d'ensemble](#vue-densemble)
2. [Les deux serveurs HTTP](#les-deux-serveurs-http)
3. [Authentification TOTP entre pairs](#authentification-totp-entre-pairs)
4. [Chiffrement des messages pair-à-pair](#chiffrement-des-messages-pair-à-pair)
5. [Le sandbox d'exécution](#le-sandbox-dexécution)
6. [Flux d'une tâche `run_remote`](#flux-dune-tâche-run_remote)
7. [Orchestration DDP avec `distribute`](#orchestration-ddp-avec-distribute)
8. [Multi-GPU par machine](#multi-gpu-par-machine)
9. [Annulation des tâches](#annulation-des-tâches)
10. [UI dispatcher](#ui-dispatcher)
11. [Streaming des logs en temps réel](#streaming-des-logs-en-temps-réel)
12. [Monitoring des ressources par tâche](#monitoring-des-ressources-par-tâche)
13. [Venv géré côté pair](#venv-géré-côté-pair)
14. [Découverte mDNS](#découverte-mdns)
15. [Privilèges et helper](#privilèges-et-helper)
16. [Modèle de sécurité](#modèle-de-sécurité)

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
    "workspace": [{"path": "train.py", "content_b64": "..."}],
    "local_id": "uuid-fourni-par-le-client"
  }
  ```
  Le handler :
  1. Récupère le code TOTP courant de l'AuthManager local
  2. Crée une entrée `OutgoingTasks` avec status `Queued` (UI visible immédiatement). Si `local_id` est fourni, c'est lui qui est utilisé comme id ; sinon UUID généré.
  3. Sur un thread `spawn_blocking` : POST vers `<peer_ip>:7655/peer/v1/tasks` avec header `X-PartaGPU-TOTP`
  4. Récupère `task_id` (du pair), enregistre `(peer_ip, remote_task_id)` dans `OutgoingTasks::remote_refs[local_id]` pour permettre une annulation ultérieure, met l'OutgoingTask en `Running`
  5. Poll `<peer_ip>:7655/peer/v1/tasks/<task_id>` toutes les 500 ms
  6. Quand la tâche atteint un état terminal (`Completed`/`Failed`/`Cancelled`), met à jour OutgoingTasks et **retourne le `Task` complet** au client
  7. Si timeout : marque comme failed et retourne 502.

  Cet endpoint est **bloquant** par design : le notebook Python attend simplement le résultat. Pour des tâches longues, on pourrait passer en async (Phase 4 du roadmap).

  La logique de dispatch est extraite en `pub fn dispatch_task_blocking()` réutilisable depuis n'importe quel contexte sync (HTTP handler via spawn_blocking, ou commande Tauri `dispatch_task` appelée par l'UI dispatcher).

- `POST /api/cancel` — **annulation d'une tâche sortante**. Body : `{"local_id": "..."}`. Look up le `remote_ref` correspondant, récupère le code TOTP courant, fait `DELETE http://<peer_ip>:7655/peer/v1/tasks/<remote_id>` puis marque l'OutgoingTask `Cancelled`. Si le pair est injoignable, marque quand même Cancelled localement et retourne 502 avec `remote: false`.

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
  Auth → résolution du `source_machine` depuis l'IP TCP source (lookup dans `discovery.get_peers()`, en préférant `display_name` puis `hostname` puis l'IP brute) → `IncomingTasks::create_and_run(...)` qui lance le sandbox dans un thread, retourne immédiatement un `task_id`. Le `source_machine` apparaît dans le tableau "Qui utilise mes ressources ?" côté pair.
- `GET /peer/v1/tasks/<id>` — auth idem, retourne la struct `Task` complète (status, output, error_output, exit_code).
- `DELETE /peer/v1/tasks/<id>` — **annulation** d'une tâche en cours. Auth idem. Marque la tâche `Cancelled` côté `IncomingTasks`, envoie `SIGTERM` au PID du bwrap, puis `SIGKILL` après 2 s si toujours en vie. Logged dans le `SecurityLog` avec `EventCategory::TaskRejected`.

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

**TOTP n'apporte que l'authenticité**, pas la confidentialité — un attaquant qui écoute le trafic en clair verrait le contenu des bodies HTTP. C'est pour ça qu'on layered un **chiffrement** par-dessus (section suivante).

---

## Chiffrement des messages pair-à-pair

Tous les bodies échangés sur le peer-API (port 7655, sauf `/peer/v1/health`) sont chiffrés en **AES-256-GCM** avec une clé dérivée du secret de salle.

### Dérivation de la clé

```
key = HKDF-SHA256(
    ikm    = base32_decode(room_secret),     // déjà partagé via la passphrase
    salt   = "PartaGPU/peer-api/v1",
    info   = "AES-256-GCM message key",
    length = 32 bytes,
)
```

Tous les membres de la salle dérivent la **même** clé puisqu'ils partagent le secret. Personne en dehors de la salle ne peut la dériver.

### Format d'enveloppe

Chaque body de requête (POST) ou de réponse (2xx) est sérialisé ainsi :

```json
{
  "v": 1,
  "nonce": "<12 octets random, base64>",
  "ct":    "<ciphertext + tag GCM, base64>"
}
```

Le Content-Type est `application/x-partagpu-encrypted-v1`.

### Ordre d'opérations côté serveur ([peer_api.rs](../src-tauri/src/peer_api.rs))

```
1. read_request                         # parse method/path/headers/body
2. si route ∈ /peer/v1/tasks* :
   - check Content-Type == ENCRYPTED_CONTENT_TYPE  (sinon 415)
   - check room_key disponible                     (sinon 415)
   - decrypt(body) -> plaintext JSON               (sinon 415)
   - replace req.body par plaintext
3. dispatch vers handle_submit / handle_get_task / handle_cancel_task
4. si status 2xx ET route encrypted :
   - encrypt(response_body, room_key) -> envelope
   - write_response avec Content-Type = ENCRYPTED_CONTENT_TYPE
5. sinon (4xx, 5xx) : envoyer en clair
```

Les erreurs (4xx, 5xx) restent en clair parce que le client peut ne pas avoir la clé (c'est ça qui a généré le 4xx). Un body 401 chiffré serait illisible.

### Ordre d'opérations côté client ([http_api.rs::run_remote_blocking](../src-tauri/src/http_api.rs))

```
1. derive_room_key depuis auth.get_secret()
2. encrypt(body) -> envelope JSON
3. ureq::post(url, Content-Type: ENCRYPTED..., X-PartaGPU-TOTP: code, body=envelope)
4. si 2xx : decrypt(response_body) -> Task JSON
   sinon : lire response.text() en clair
```

### Propriétés

- **Confidentialité** : un attaquant qui écoute le trafic LAN ne peut rien lire (script Python, données workspace, stdout/stderr).
- **Intégrité** : tout flip de bit dans le ciphertext fait échouer le déchiffrement (tag GCM rejeté). Le serveur retourne 415.
- **Authenticité au niveau salle** : seul un détenteur du secret peut produire un envelope qui se déchiffre proprement. Combiné au TOTP, on a auth + intégrité + replay-protect (TOTP fenêtre 30 s).

### Hors scope

- **Forward secrecy** : pas d'échange de clé éphémère. Si le secret est leaké un jour, tout l'historique enregistré devient lisible.
- **Protection contre un membre de la salle** : par construction, tout pair dans la salle a la clé. Le modèle de menace est "attaquant LAN qui n'est PAS dans la salle".
- **Compatibilité ascendante** : avant `1.6.0`, les bodies passaient en clair. Les pairs en `< 1.6.0` ne peuvent plus parler à des pairs en `>= 1.6.0`. Upgrade simultané requis.

### Tests

`crypto.rs` a des tests unitaires : round-trip, mauvaise clé, ciphertext altéré, JSON round-trip (`cargo test --lib crypto::`).

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

## Annulation des tâches

Une tâche en cours peut être annulée à tout moment, et l'annulation se propage proprement bout-à-bout : du client qui demande au sandbox du pair qui doit s'arrêter.

### Tracking des PIDs côté pair

`IncomingTasks` maintient un map `pids: HashMap<task_id, u32>` des PIDs des bwrap en cours. Le PID est enregistré via le callback `on_pid` de `Sandbox::execute_with_callbacks` (appelé juste après le `spawn`), et retiré quand le wait loop se termine (process mort).

### `IncomingTasks::cancel(task_id)`

```
1. Marque la tâche `Cancelled` AVANT le SIGTERM (ordre important — voir
   ci-dessous).
2. Envoie `kill -TERM <pid>` (commande shell, pas libc — évite la
   dépendance directe à libc).
3. Spawn un thread qui dort 2 s puis fait `kill -KILL <pid>` si la tâche
   est toujours dans le map de PIDs (cas où SIGTERM est ignoré, ex:
   tâche avec un handler de signal).
4. Le wait loop dans le thread d'exécution voit le bwrap mourir,
   capture stdout/stderr/exit_code (typiquement exit 143 = 128+SIGTERM),
   appelle le completion handler.
5. Le completion handler détecte `task.status == Cancelled` (déjà mis
   en (1)) et n'override PAS le status avec Failed. Il met juste à
   jour les outputs et exit_code.
```

L'ordre (1)→(2) est crucial : si on faisait SIGTERM avant de marquer Cancelled, le wait loop pourrait revenir AVANT qu'on ait écrit Cancelled, et le completion handler écrirait `Failed` (puisqu'exit != 0).

### Annulation côté client

`OutgoingTasks::remote_refs: HashMap<local_id, RemoteRef>` où `RemoteRef = { peer_ip, remote_task_id }`. Renseigné par `dispatch_task_blocking` après que le pair a accepté la tâche.

`http_api::cancel_outgoing_task(auth, outgoing, local_id)` (fonction sync, réutilisable) :
1. Look up `remote_ref` ; si absent → la tâche n'a jamais atteint le pair (ou est déjà terminée), juste marquer `Cancelled` localement et retourner.
2. Récupérer le code TOTP courant.
3. `ureq::delete("http://<peer_ip>:7655/peer/v1/tasks/<remote_id>", X-PartaGPU-TOTP: code)`.
4. Si le pair répond 2xx → marquer `Cancelled` côté local, retourner `Ok(true)`.
5. Si erreur réseau → marquer `Cancelled` localement quand même (le user a exprimé son intent), retourner `Err`.

### Propagation depuis Python

Pour que `Ctrl+C` dans un notebook annule la tâche distante, le client Python doit savoir le `local_id` AVANT que `requests.post(/api/dispatch)` ne retourne. Solution : le client **pré-alloue** un UUID côté Python et le passe dans le body de dispatch :

```python
local_id = str(uuid.uuid4())
try:
    requests.post("/api/dispatch", json={..., "local_id": local_id})
except KeyboardInterrupt:
    requests.post("/api/cancel", json={"local_id": local_id})
    raise
```

`partagpu.run_remote()` fait exactement ça en interne. Si `local_id` est fourni dans le body, l'app utilise cet id pour l'OutgoingTask au lieu d'en générer un.

### Annulation des rangs siblings dans `distribute()`

Quand un rang plante au milieu d'un entraînement DDP, les autres restent bloqués sur l'`init_process_group` ou un `all-reduce`, en attente du rang mort, jusqu'à atteindre le timeout NCCL (~30 min par défaut). Pour éviter ça, `distribute()` :

1. Pré-alloue un `local_id` par rang.
2. Lance les workers en parallèle via `ThreadPoolExecutor`.
3. Sur le **premier** rang qui retourne avec `TaskResult.ok == False` ou qui lève une exception, appelle `partagpu.cancel(local_id)` sur tous les autres rangs encore en cours.
4. Sur `KeyboardInterrupt` dans le main thread : annule **tous** les rangs avant de re-raise.

Les rangs annulés retournent un `TaskResult` avec `status="Cancelled"`. Le caller voit donc tous les résultats (un Failed, plusieurs Cancelled), pas une exception.

### Bouton Stop dans l'UI

Le composant `TaskList` rend un bouton **Stop** sur chaque tâche en `Queued` ou `Running`. Selon `direction`:
- `incoming` → appelle la commande Tauri `cancel_incoming_task(task_id)` qui invoque `IncomingTasks::cancel()`.
- `outgoing` → appelle `cancel_outgoing_task(local_id)` qui invoque `http_api::cancel_outgoing_task()` (la même fonction que `POST /api/cancel`, juste appelée directement sans HTTP).

---

## UI dispatcher

`src/components/TaskDispatcher.tsx` est un formulaire React qui permet de dispatcher une commande sur un pair sans passer par le package Python. Visible dans l'onglet *Mon utilisation*.

### Flux

1. L'utilisateur sélectionne un pair (dropdown peuplé depuis `getPeers`, filtré sur `verified && sharing_enabled`), tape une commande (parsée en argv via un mini-parseur shell qui gère `'…'`, `"…"`, `\\`), choisit un timeout, optionnellement coche "réseau autorisé".
2. Click sur **Lancer** → invoke `dispatch_task` (commande Tauri).
3. La commande Tauri appelle `http_api::dispatch_task_blocking()` (la même fonction que `POST /api/dispatch`, juste sans la couche HTTP). C'est sync (s'exécute sur le thread pool Tauri), donc le `await` côté JS attend la fin réelle de la tâche côté pair.
4. Le `Task` final est rendu dans le panneau résultat : badge status, exit_code, stdout/stderr en `<pre>` collapsibles.

### Pourquoi un Tauri command et pas un fetch direct vers /api/dispatch ?

Pour éviter une self-loopback HTTP qui ajouterait une latence et une surface d'erreur inutile. Comme l'UI tourne dans le même process que `dispatch_task_blocking`, l'invoke direct est plus propre.

### Workspace upload

Le formulaire inclut une section **Fichiers du workspace** : un file picker multi-fichiers, plus une liste des fichiers sélectionnés avec leur taille et un bouton de suppression. Au lancement, chaque fichier est lu en `ArrayBuffer` côté JS, encodé base64 (par chunks de 32 KB pour éviter le stack overflow de `String.fromCharCode.apply`), et passé via le param `workspace` du Tauri command. Total cappé à 16 MB côté UI (warning si dépassé) et côté sandbox.

Le user référence un fichier dans la commande par son nom de base : par ex. après upload de `train.py`, taper la commande `python3 train.py` lance le script poussé.

### Limites volontaires

- **Pas de DDP** depuis l'UI. L'UI dispatcher est pour des tâches single-worker. DDP reste l'API Python `partagpu.distribute(...)`.
- **Pas d'arborescence dans le workspace** depuis l'UI (uniquement des fichiers à plat). Pour pousser un sous-dossier, utilisez `partagpu.run_remote(..., workspace={"sub/file.py": "..."})` côté Python.

---

## Streaming des logs en temps réel

Lecture incrémentale de stdout/stderr pendant l'exécution, sans attendre la fin du process. Permet de voir les `print()` d'un long entraînement défiler dans l'UI au fur et à mesure.

### Côté sandbox

Le sandbox lit stdout/stderr du bwrap via deux **threads readers** dédiés (`drain_stream`), qui consomment des chunks de 4 KB et les append à des buffers partagés `Arc<Mutex<String>>`. Ces buffers sont :
- Soit **internes au sandbox** si aucun observateur n'est branché (cas d'usage stand-alone, équivalent à l'ancien comportement).
- Soit **fournis par le caller** via le struct `OutputSink { stdout, stderr }` passé à `execute_with_callbacks_and_sink(...)`.

Les readers respectent un cap (1 MB stdout, 256 KB stderr — configurable via `MAX_STDOUT_BYTES` / `MAX_STDERR_BYTES`) et gèrent les multi-bytes UTF-8 coupés en fin de chunk (carry-over vers le prochain).

À la fin de l'exécution, après `wait_with_timeout`, le sandbox **join** les threads readers pour garantir que tous les bytes sont capturés avant de retourner le `SandboxResult`.

### Côté `IncomingTasks`

Map `sinks: HashMap<task_id, OutputSink>` :
- `spawn_execution` crée un `OutputSink` AVANT le `execute_*`, l'inscrit dans la map, le passe au sandbox.
- `get(id)` et `list()` lisent ce sink (snapshot des buffers via `OutputSink::snapshot()`) si la tâche est encore en cours, et écrasent `task.output` / `task.error_output` dans le `Task` retourné. Si la tâche est déjà terminée (sink retiré), on retourne le `Task` tel quel.
- Le sink est retiré de la map dès que le wait_loop revient.

Résultat : un `GET /peer/v1/tasks/<id>` retourne toujours l'output partiel le plus à jour, qu'il s'agisse d'une tâche en cours ou terminée.

### Côté `OutgoingTasks` (machine de lancement)

`update_outputs(local_id, stdout, stderr)` copie l'output partiel d'une tâche distante dans son miroir local. Appelée à chaque tick (~500 ms) du poll loop dans `run_remote_blocking` :

```
loop:
  GET /peer/v1/tasks/<remote_id>     # récupère un Task complet avec output partiel
  outgoing.update_outputs(local_id, task.output, task.error_output)
  if task.status == terminal: return task
  sleep 500ms
```

### Côté UI

Pendant qu'un dispatch est en cours, le composant `TaskDispatcher` :
1. Pré-alloue un `local_id` (UUID) et le passe à la commande Tauri `dispatch_task`.
2. Démarre un `setInterval(500ms)` qui appelle `getOutgoingTasks()` et trouve la tâche par cet id, puis met à jour un `livePartial` state.
3. Le panneau résultat affiche `displayedTask = result ?? livePartial` : avant la fin du dispatch, on voit l'output partiel grandir ; après, le `result` final remplace.
4. Stop l'interval quand l'invoke résout.

### Pourquoi `dispatch_task` est `async`

Si `dispatch_task` était sync, Tauri exécuterait sa logique sur le thread IPC principal — bloqué pour toute la durée de la tâche. Pendant ce temps, `getOutgoingTasks` queue, le polling de `livePartial` ne tourne pas, et l'UI gèle (potentiellement avec un message OS "ne répond pas"). En async + `tokio::task::spawn_blocking` pour la partie ureq, le thread IPC reste libre, le polling continue, l'output défile en direct.

Idem pour `cancel_outgoing_task` qui fait aussi du ureq sync vers le pair.

### Buffering Python à connaître

Côté script utilisateur, `print()` est par défaut **block-buffered** quand stdout n'est pas un TTY (ce qui est notre cas : pipe vers le bwrap parent). Tout est gardé en mémoire jusqu'à un `flush()`, un newline en line-buffered mode, ou la fermeture du process. Pour voir les `print()` défiler en direct :
- `print(..., flush=True)` à chaque appel
- ou `python3 -u` (unbuffered)
- ou `PYTHONUNBUFFERED=1` dans l'environnement (déjà passé par notre sandbox… non, pas par défaut, à ajouter si on veut)

Le script `examples/ddp_train_demo.py` utilise déjà `print(..., flush=True)`. Le sandbox **force aussi `PYTHONUNBUFFERED=1`** dans l'environnement de chaque tâche (cf. [sandbox.rs](../src-tauri/src/sandbox.rs)), donc les `print()` sans `flush=True` arrivent quand même en direct.

---

## Monitoring des ressources par tâche

Pour que l'UI montre une **progression** qui avance et des **valeurs CPU/RAM** réelles pendant qu'une tâche tourne (au lieu d'un saut 0% → 100% à la fin), `IncomingTasks` lance un **thread monitor** au démarrage qui tourne toute la vie de l'app.

### Boucle

```
loop forever:
  sleep 1s
  sysinfo::System.refresh_processes(All)
  pour chaque (task_id, bwrap_pid) dans pids:
    tree = collect_descendants(sysinfo, bwrap_pid)
    cpu_total = somme des process.cpu_usage() du tree
    ram_total = somme des process.memory() du tree
    progress = clamp((elapsed / timeout) * 100, 0..99)
    si task.status == Running:
      task.cpu_usage = cpu_total
      task.ram_usage_mb = ram_total
      task.progress = progress
```

### Détails

- **Process tree** : `bwrap` est le parent direct, mais c'est `python3` (et ses propres enfants éventuels) qui consomme la majorité du CPU/RAM. Une fonction `collect_descendants` parcourt en BFS la map des processus de sysinfo et retient tout ce qui descend du PID bwrap. La somme inclut donc bwrap + python + tout petit-enfant.

- **Progression = elapsed/timeout** : pas de mesure intrinsèque "30% du job" possible pour une commande arbitraire ; on utilise donc le ratio temps écoulé / timeout, capé à 99 % jusqu'à ce que la tâche atteigne réellement un état terminal. Approximation imparfaite mais visible et utile.

- **GPU per-task** : pas implémenté dans cette version. `nvidia-smi` ne donne pas la consommation GPU par PID directement (il faudrait NVML via `nvml-wrapper` ou un cgroup v2 GPU resource controller). La colonne GPU des tâches reste à 0 % ; la jauge globale "Ressources de cette machine" continue de remonter le GPU usage agrégé.

- **`task_starts` + `task_timeouts`** : deux maps `HashMap<task_id, _>` dans `IncomingTasks`, peuplées dans `spawn_execution` quand la tâche transitionne en Running, et nettoyées à la fin du thread d'exécution.

### Côté machine de lancement (OutgoingTasks)

`run_remote_blocking` poll le pair toutes les 500 ms. À chaque tick, en plus de mirror le stdout/stderr, il copie aussi `progress`, `cpu_usage`, `ram_usage_mb`, `gpu_usage` du Task distant dans le miroir local. Méthode dédiée : `OutgoingTasks::mirror_running(local_id, &peer_task)`.

Résultat : l'UI de la machine de lancement (page *Mon utilisation*) voit les mêmes valeurs live que l'UI de la machine cible (page *Mon partage*).

### Répartition par utilisateur

La page *Mon partage* affiche un panneau **Répartition par utilisateur** qui empile les conso CPU/RAM/GPU des tâches courantes par `source_user`. Avec le monitoring temps réel, ce panneau est désormais peuplé en direct au lieu de rester à 0 %. Couleurs distinctes par user (jusqu'à 8). C'est ce qu'un prof regarde pour voir quel élève saturate la machine.

---

## Venv géré côté pair

PartaGPU peut provisionner un venv Python pré-rempli (`torch`, `numpy`) sur chaque machine, pour éviter à l'utilisateur de faire `sudo pip install --break-system-packages …` côté système (qui ne marche que pour le user `partagpu`, demande un mot de passe sudo, et pollue le Python système).

### Provisionnement

Le helper privilégié expose deux sous-commandes :

```
sudo /usr/local/lib/partagpu/partagpu-helper setup-venv
sudo /usr/local/lib/partagpu/partagpu-helper remove-venv
```

`setup-venv` :
1. Crée `/var/lib/partagpu/venv` via `python3 -m venv`.
2. Met à jour pip dans le venv.
3. Installe `torch` + `numpy` (best effort — torch est le gros download, ~2 Go).
4. `chown -R partagpu:partagpu /var/lib/partagpu/venv` pour que la sandbox UID puisse y lire.

`remove-venv` : `rm -rf /var/lib/partagpu/venv`.

### Côté UI

Page *Mon partage* → section *Environnement Python pour les tâches reçues*. Composant [`ManagedVenvPanel`](../src/components/ManagedVenvPanel.tsx) :
- Status (installé / non installé) + chemin
- Bouton **Installer torch + numpy (~2 Go)** → invoke `setup_managed_venv` async (qui lance le helper via pkexec)
- Bouton **Mettre à jour** (re-run de l'install pour upgrader)
- Bouton **Supprimer** → invoke `remove_managed_venv`

Les commandes Tauri sont **async** (`tokio::task::spawn_blocking` autour du pkexec) parce que l'install de torch peut bloquer 5-10 minutes. Sans ça, l'UI gèlerait pendant le download (cf. la même leçon que pour [dispatch_task](#ui-dispatcher)).

### Côté sandbox

Quand `bwrap` lance une tâche, il :
1. **Bind-mount `/var/lib/partagpu/venv` (host) → `/opt/partagpu-venv` (sandbox), read-only**, si le dossier existe.
2. **Override `PATH`** : `/opt/partagpu-venv/bin:/usr/local/bin:/usr/bin:/bin`. Les arguments de la tâche qui invoquent `python3` (basename, pas chemin absolu) sont résolus via PATH → ils trouvent le binaire du venv en premier.
3. **Force `PYTHONUNBUFFERED=1`** dans l'env (utile pour le streaming, cf. section précédente).

Si le venv n'est pas installé, le sandbox tombe sur le `python3` système comme avant — comportement rétro-compatible. Les utilisateurs qui ont déjà installé torch en system Python continuent de l'utiliser.

### Pourquoi un venv plutôt que `pip install --break-system-packages` automatique

- **Pas de pollution** du Python système. L'utilisateur garde le contrôle de son `/usr/lib/python3/dist-packages` pour ses propres outils.
- **Versionnable** : on peut un jour décider de bouger `torch==2.x` à `torch==2.y` proprement, sans risquer de casser un autre outil système qui dépend d'une version particulière.
- **Désinstallable d'un seul `rm -rf`**, sans casser apt.
- **Multi-utilisateur** : tous les utilisateurs locaux qui lancent l'app PartaGPU partagent le même venv (vu via le sandbox), pas un par user.

### Limites actuelles

- Liste fixe de packages : `torch + numpy`. Pour ajouter (`pandas`, `scikit-learn`, `transformers`…), il faut soit éditer le helper, soit que l'utilisateur fasse `sudo /var/lib/partagpu/venv/bin/pip install <package>` à la main. Pas d'UI pour ajouter / retirer un package — viendrait dans une itération future.
- Pas d'indicateur de progression pendant l'install (pkexec masque le stdout du helper). L'UI affiche un spinner et le terminal de `npm run tauri:dev` montre la sortie pip si on veut suivre.
- Pas d'auto-update. Si torch sort une nouvelle version, l'utilisateur doit cliquer "Mettre à jour".

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
