# Guide utilisateur — Package Python `partagpu`

Comment **vraiment se servir** du package Python pour exploiter les GPU partagés d'une salle, depuis l'installation jusqu'à un entraînement DDP réel. Pour le détail interne du flux (TOTP, sandbox, etc.), voir [ARCHITECTURE.md](ARCHITECTURE.md).

---

## Table des matières

1. [Préparation côté machines](#préparation-côté-machines)
2. [Installation du package](#installation-du-package)
3. [Premier appel : `discover`](#premier-appel-discover)
4. [Une commande sur un pair : `run_remote`](#une-commande-sur-un-pair-run_remote)
5. [Entraînement DDP : `distribute`](#entraînement-ddp-distribute)
6. [Multi-GPU sur une même machine](#multi-gpu-sur-une-même-machine)
7. [Diagnostic : que faire si ça ne marche pas](#diagnostic-que-faire-si-ça-ne-marche-pas)
8. [Smoke tests](#smoke-tests)
9. [Référence rapide](#référence-rapide)

---

## Préparation côté machines

Pour qu'un pair puisse **recevoir** des tâches Python (run_remote ou distribute), il faut sur **chaque machine cible** :

1. **L'application PartaGPU lancée et dans la même salle** que vous (même nom de salle, même passphrase 4 mots).
2. **Le partage activé** (toggle "Mon partage" → Actif).
3. **`bubblewrap` installé** :
   ```bash
   sudo apt install -y bubblewrap
   ```
4. **Pour DDP uniquement : `torch` (et `numpy`) en Python système**, pas dans un venv utilisateur :
   ```bash
   sudo apt install -y python3-pip
   sudo /usr/bin/python3 -m pip install --break-system-packages torch numpy
   ```
   Pourquoi : le sandbox de PartaGPU tourne sous l'UID `partagpu`, qui ne peut pas voir votre `~/.local` ni les venvs utilisateur. Seuls les packages dans `/usr/lib/python3/dist-packages/` ou `/usr/local/lib/python3.*/dist-packages/` sont accessibles.

Sur la **machine de lancement** (celle où vous écrivez votre script Python), torch et le package `partagpu` peuvent en revanche être dans un venv : c'est votre Python à vous, pas celui du sandbox. Voir l'install ci-dessous.

---

## Installation du package

Le package n'est **pas encore sur PyPI**. Installation depuis le clone du repo :

```bash
git clone https://github.com/cesar-lizurey/partagpu.git
cd partagpu

# venv recommandé (sinon tout va en system Python — plus sale)
python3 -m venv venv
source venv/bin/activate
pip install -e python/        # mode éditable : suit l'état du repo

# Pour les exemples
pip install ipykernel requests numpy torch     # numpy/torch optionnels
```

Le mode éditable (`-e`) signifie qu'un `git pull` met immédiatement à jour les fonctions importées — pas de re-install nécessaire pour les changements Python.

Pour utiliser dans un Jupyter notebook :
```bash
python -m ipykernel install --user --name=partagpu --display-name="Python (PartaGPU)"
```
puis sélectionner ce kernel dans Jupyter.

---

## Premier appel : `discover`

```python
import partagpu

gpus = partagpu.discover()
for g in gpus:
    print(g)
```

Sortie typique avec 2 PC dans la salle, chacun avec 1 GPU :
```
GPU('local',   ip='192.168.70.103', dev=0, limit=100.0%, verified)
GPU('César 2', ip='192.168.70.105', dev=0, limit=50.0%,  verified)
```

Ce que ça veut dire :
- `local` est votre machine. `limit=100%` car vous ne limitez pas votre propre usage.
- `César 2` est un pair vérifié (TOTP OK) qui partage 50% de son GPU.
- `dev=0` est l'index du GPU physique sur cette machine. Un PC avec 4 GPU produit 4 entrées (`dev=0`, `dev=1`, etc.).

Si la liste est vide ou incomplète :
- L'app tourne-t-elle ? `curl -s http://127.0.0.1:7654/api/status`
- Êtes-vous dans une salle ? UI → onglet *Mon partage*, en haut.
- Les pairs sont-ils dans la **même** salle (même passphrase) ? Sinon ils apparaissent comme `unverified` et ne sont pas listés par `discover()`.
- Les pairs partagent-ils ? Le toggle "Activer le partage" doit être ON sur leur PC.

---

## Une commande sur un pair : `run_remote`

Cas d'usage : "je veux exécuter un script ponctuel sur la GPU d'un pair, et récupérer le stdout".

### Exemple minimal

```python
import partagpu

# Choisir un pair (n'importe lequel sauf 'local' si vous voulez vraiment du distant)
peer = next(g for g in partagpu.discover() if g.host != "local")

result = partagpu.run_remote(
    peer,
    ["python3", "-c", "import torch; print(torch.cuda.get_device_name(0))"],
    timeout=30,
)
print(result.stdout)
result.check()         # raise RemoteTaskError si exit != 0 ou status != Completed
```

### Paramètres importants

| Argument | Effet |
|---|---|
| `peer` | un `GPUResource` ou une IP (`str`). Pas besoin de SSH ni de connexion directe : tout passe par votre app locale. |
| `args` | liste de strings — le sandbox utilise une allowlist de commandes (`python3`, `nvidia-smi`, `bash`, etc.). Pas de shell. |
| `timeout=300` | wall-clock cap, en secondes. Couvre l'exécution + le polling local. |
| `network=False` | `True` lève `--unshare-net` du sandbox. Requis pour DDP, sinon laissez à False (par défaut). |
| `workspace={...}` | `dict` de `{path: contenu}` ou liste de `Path` à pousser dans le `/workspace` du sandbox avant exec. Total ≤ 16 Mo. |
| `user="alice"` | label informatif visible dans l'UI du pair (panneau "tâches entrantes"). |

### Ce que retourne `TaskResult`

```python
@dataclass
class TaskResult:
    id: str                 # UUID de la tâche côté pair
    target_machine: str     # display_name du pair (ou son IP)
    status: str             # "Completed" | "Failed" | "Cancelled"
    exit_code: int | None
    stdout: str             # cappé à 1 Mo
    stderr: str             # cappé à 256 Ko
    @property ok            # status == "Completed" and exit_code == 0
    def check()             # raise RemoteTaskError si pas OK
```

### Erreurs courantes

- `RemoteTaskError: Dispatch refusé (HTTP 412) : Cette machine n'est dans aucune salle PartaGPU.` → vous n'êtes pas dans une salle. UI → créer/rejoindre une salle.
- `RemoteTaskError: Le pair ... a refusé la tâche (HTTP 401) : Code TOTP invalide ou expiré.` → décalage d'horloge entre vos deux PC > 30 s. Synchronisez via `sudo timedatectl set-ntp true` sur chaque machine.
- `Commande refusée : « X » n'est pas dans la liste autorisée` → commande pas dans l'allowlist du pair. UI → onglet *Mon partage* du pair → *Allowlist* → ajouter X.
- `Failed to initialize NumPy: No module named 'numpy'` (warning torch) → installer `numpy` côté system Python du pair.

### Pousser un script (workspace)

Pour exécuter un fichier `.py` qui n'existe pas chez le pair, on le pousse dans le `/workspace` :

```python
result = partagpu.run_remote(
    peer,
    ["python3", "train.py", "--epochs", "10"],
    workspace=["./train.py", "./config.yaml"],   # liste de Path
    network=True,                                 # pour le rendezvous DDP
    timeout=1800,
)
```

Côté pair, `train.py` et `config.yaml` apparaissent dans `/workspace` (le cwd du sandbox). Ils sont nettoyés à la fin de la tâche.

---

## Entraînement DDP : `distribute`

Cas d'usage : "je veux entraîner mon modèle PyTorch sur **tous les GPU de la salle**, en parallèle, avec NCCL all-reduce."

C'est ce pour quoi PartaGPU est fait. `distribute()` fait **automatiquement** :
- la découverte des GPU
- l'upload du script à chaque pair
- la configuration des env vars DDP (MASTER_ADDR, RANK, WORLD_SIZE, ...)
- le lancement parallèle des workers
- la collecte des résultats

### Exemple minimal

`train.py` :
```python
import os
import torch
import torch.nn as nn
import torch.distributed as dist
from torch.nn.parallel import DistributedDataParallel as DDP

dist.init_process_group(backend=os.environ["BACKEND"], init_method="env://")
rank = int(os.environ["RANK"])
world_size = int(os.environ["WORLD_SIZE"])

device = torch.device("cuda:0")  # CUDA_VISIBLE_DEVICES vous a déjà filtré au bon GPU

model = nn.Linear(784, 10).to(device)
model = DDP(model)

# ... votre boucle d'entraînement habituelle
# (le DDP synchronise les gradients automatiquement)

dist.destroy_process_group()
```

Lancement :
```python
import partagpu

results = partagpu.distribute(
    "train.py",
    args=["--epochs", "10"],
    timeout=1800,            # 30 min max
)
for r in results:
    print(r.target_machine, "exit", r.exit_code)
    print(r.stdout[-500:])    # les 500 derniers caractères du stdout
```

### Paramètres importants

| Argument | Effet |
|---|---|
| `script` | chemin vers le `.py`. Lu et envoyé en workspace à chaque pair. |
| `args=()` | args appendus à `python3 <script>`. |
| `gpus=None` | par défaut : tous les GPU de `discover()`. Sinon liste explicite (ex: `[gpus[0], gpus[2]]` pour ne prendre que 2 sur 4). |
| `extra_files=()` | autres fichiers à uploader dans le workspace (`config.yaml`, `model.py`, etc.). |
| `master_port=29500` | port du rendezvous NCCL/Gloo. Doit être dans la range 29500–29510 (ouverte par le helper firewall). |
| `backend="nccl"` | `"nccl"` (GPU) ou `"gloo"` (CPU/GPU). Default NCCL. |
| `timeout=3600` | wall-clock cap par worker, en secondes. |

### Variables d'environnement reçues par `train.py`

| Var | Valeur | Usage typique |
|---|---|---|
| `RANK` | rang global, 0..world_size-1 | identification du process dans le groupe DDP |
| `WORLD_SIZE` | nombre total de processus | passé à `init_process_group` (implicite via `init_method='env://'`) |
| `LOCAL_RANK` | toujours `0` (CVD filtre) | `torch.cuda.set_device(LOCAL_RANK)` ou `cuda:LOCAL_RANK` reste correct |
| `PARTAGPU_LOCAL_RANK` | position parmi les workers du même host | logging multi-GPU par machine |
| `MASTER_ADDR` | IP du rang 0 | rendezvous |
| `MASTER_PORT` | port du rendezvous (29500 par défaut) | rendezvous |
| `BACKEND` | "nccl" ou "gloo" | `init_process_group(backend=os.environ["BACKEND"])` |
| `CUDA_VISIBLE_DEVICES` | index physique du GPU assigné | torch ne voit qu'un seul GPU = `cuda:0` |

### DistributedSampler

Si vous utilisez un `Dataset` côté Python, **n'oubliez pas le `DistributedSampler`** sinon chaque rang voit toutes les données :

```python
from torch.utils.data import DataLoader, DistributedSampler

sampler = DistributedSampler(dataset, num_replicas=world_size, rank=rank, shuffle=True)
loader = DataLoader(dataset, batch_size=32, sampler=sampler)

for epoch in range(epochs):
    sampler.set_epoch(epoch)   # important pour le shuffling
    for batch in loader:
        ...
```

### Erreurs courantes

- `ModuleNotFoundError: No module named 'torch'` (du sandbox du pair) → `torch` n'est pas dans le system Python du pair. `sudo /usr/bin/python3 -m pip install --break-system-packages torch numpy` sur la machine cible.
- `RuntimeError: Connection reset by peer` au `init_process_group` → port 29500 pas joignable depuis l'autre machine. Vérifier que le helper a ouvert TCP 29500–29510 (devrait être automatique au sharing enable). Sinon : `sudo ufw allow 29500:29510/tcp`.
- `CUDA error: invalid device ordinal` → un script qui fait `cuda:1` alors que `CUDA_VISIBLE_DEVICES` filtre à un seul GPU. Utilisez `cuda:0` ou `cuda:LOCAL_RANK` (qui vaut 0).
- DDP qui hang à `init_process_group` puis timeout → décalage d'horloge entre les PC > 30 s ; OU un rang n'a jamais démarré (regardez ses logs dans l'UI). Solution : vérifiez que `partagpu.run_remote(peer, ["python3", "-c", "print('hi')"])` marche bien sur **chaque** pair avant de lancer DDP.

---

## Multi-GPU sur une même machine

Si une machine a 4 GPU physiques, `discover()` produira **4 entrées distinctes** pour cette machine, même IP, `device_index` 0..3. `distribute()` lancera donc 4 workers sur cette machine, chacun isolé à son propre GPU via `CUDA_VISIBLE_DEVICES`.

Côté `train.py`, **rien à changer** : chaque worker voit son GPU comme `cuda:0`.

### Tester sans avoir un vrai serveur multi-GPU

```bash
PARTAGPU_FORCE_GPU_COUNT=4 npm run tauri:dev
```

Ça simule 4 GPU dans l'app, donc `discover()` retournera 4 entrées locales. `distribute()` lancera 4 workers... qui essaieront tous d'utiliser des GPU physiques 0/1/2/3 dont seulement 0 existe vraiment. Donc :
- Test d'upload + env vars : OK (utiliser un script "probe" qui imprime `os.environ`, comme `examples/smoke_multi_gpu.py`)
- Test DDP réel : worker 0 marche, workers 1/2/3 plantent au `init_process_group`. C'est attendu : c'est juste un test de logique de dispatch, pas du DDP réel.

---

## Diagnostic : que faire si ça ne marche pas

### `discover()` retourne 0 GPU

```bash
# 1. L'app est-elle lancée ?
curl -s http://127.0.0.1:7654/api/status

# 2. mDNS voit-il les pairs ?
curl -s http://127.0.0.1:7654/api/peers | python3 -m json.tool

# 3. Les pairs partagent-ils ?
# Dans la sortie ci-dessus, vérifier que verified=true ET sharing_enabled=true pour les pairs.
```

Si verified=false : code TOTP qui ne match pas. Causes :
- Pas dans la même salle (différentes passphrase). Quitter et rejoindre avec la bonne.
- Décalage d'horloge > 30 s. `sudo timedatectl set-ntp true` sur chaque PC.

Si sharing_enabled=false : le pair n'a pas activé son toggle. Le lui demander.

### `run_remote` plante avec "RemoteTaskError: Dispatch refusé (HTTP 412)"

→ Vous n'êtes dans aucune salle. UI → onglet en haut → *Créer/Rejoindre une salle*.

### Le sandbox plante avec `Permission non accordée (os error 13)`

Bug fixé en `1.1.0+`. `git pull && npm run tauri:dev` côté pair pour mettre à jour.

### Une tâche reste en `Queued` indéfiniment

→ Le sandbox n'arrive pas à se lancer. Causes :
- `bubblewrap` pas installé : `sudo apt install bubblewrap`
- Le helper n'a pas créé le user partagpu : *Mon partage* → *Désactiver* puis *Activer* (relance la création du user).

### NCCL hang au `init_process_group`

→ Le port 29500 n'est pas joignable entre les machines. Tester :
```bash
# Depuis la machine A, vers la machine B
nc -zv 192.168.x.y 29500
```
Si ça échoue : firewall. Vérifier `sudo ufw status` ou `sudo iptables -L INPUT | grep 29500`. Si rien : le helper n'a pas ouvert le port. Re-faire un toggle off/on du partage.

### Logs du sandbox côté pair

Sur la machine cible :
```bash
journalctl --user -u partagpu -n 50    # si l'app tourne en mode service
# OU regarder le terminal où npm run tauri:dev s'exécute
```

---

## Smoke tests

Trois scripts dans [`examples/`](../examples/) à exécuter dans l'ordre :

```bash
cd examples
source venv/bin/activate     # ou ./venv/bin/python directement
```

### `smoke_run_remote.py` — Validation MVP

Loopback : votre PC dispatch vers lui-même.

```bash
python smoke_run_remote.py
```

Vérifie :
1. App reachable + dans une salle + partage actif
2. Peer API joignable sur 7655
3. Une commande Python s'exécute dans le sandbox
4. Une commande qui plante (exit 7) est bien marquée Failed
5. Une commande hors allowlist est rejetée

### `smoke_ddp.py` — DDP `world_size=1` puis multi-machine

```bash
# Local seul (Tests 1-3)
python smoke_ddp.py

# Avec un 2e PC dans la salle (Tests 1-4)
PARTAGPU_TEST_MULTI=1 python smoke_ddp.py
```

Vérifie :
1. Workspace upload + network_enabled (sans torch)
2. `import torch` + `cuda.is_available()` dans le sandbox
3. `distribute()` avec 1 worker (NCCL trivial)
4. (opt-in) `distribute()` avec 2 workers sur 2 PC, vrai NCCL all-reduce

### `smoke_multi_gpu.py` — Logique multi-GPU par host

Nécessite `PARTAGPU_FORCE_GPU_COUNT=N` au lancement de l'app pour simuler N GPU.

```bash
# Stop l'app actuelle, relancer avec :
PARTAGPU_FORCE_GPU_COUNT=2 npm run tauri:dev

# Dans un autre terminal :
python smoke_multi_gpu.py
```

Vérifie que les env vars (`RANK`, `LOCAL_RANK`, `PARTAGPU_LOCAL_RANK`, `CUDA_VISIBLE_DEVICES`) sont correctement attribuées à chaque worker.

---

## Référence rapide

```python
# Imports
import partagpu
from partagpu import GPUResource, TaskResult, RemoteTaskError

# Découverte
gpus = partagpu.discover()   # list[GPUResource]

# Une commande
r = partagpu.run_remote(
    peer,                          # GPUResource | str (IP)
    args,                          # list[str]
    timeout=300,                   # seconds
    user=None,                     # str | None
    network=False,                 # bool
    workspace=None,                # dict[str, str|bytes] | list[str|Path] | None
    api_base="http://127.0.0.1:7654",
) -> TaskResult

# DDP
results = partagpu.distribute(
    script,                        # str | Path
    args=(),                       # Sequence[str]
    gpus=None,                     # list[GPUResource] | None (default: all)
    extra_files=(),                # Sequence[str | Path]
    master_port=29500,             # int (must be in 29500..29510)
    backend="nccl",                # "nccl" | "gloo"
    timeout=3600,                  # seconds per worker
    user=None,
    api_base="http://127.0.0.1:7654",
) -> list[TaskResult]   # ordered by RANK

# TaskResult fields
r.id, r.target_machine, r.status, r.exit_code, r.stdout, r.stderr
r.ok                  # property: status == "Completed" and exit_code == 0
r.check()             # raise RemoteTaskError if not ok
```

---

## Liens utiles

- [README principal](../README.md) — installation et utilisation de l'app
- [ARCHITECTURE.md](ARCHITECTURE.md) — comment ça marche en interne
- [SECURITY.md](../SECURITY.md) — modèle de sécurité
- [Notebook d'exemples](../examples/decouverte_gpu.ipynb)
- [Smoke tests](../examples/)
