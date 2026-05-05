# partagpu

Client Python pour [PartaGPU](https://github.com/cesar-lizurey/partagpu) — utilisez les GPU de plusieurs machines d'une salle de cours pour l'entraînement distribué.

## Installation

```bash
pip install partagpu
```

## Pré-requis

L'application PartaGPU doit tourner sur votre machine *et* sur chaque pair, toutes dans la **même salle** (même code d'accès), avec le **partage activé** sur les pairs cibles.

Pour DDP : `torch` doit être installé dans le **Python système** (`/usr/bin/python3`) de chaque pair — un venv utilisateur ne suffit pas (le sandbox tourne sous l'utilisateur `partagpu` et ne voit pas votre `$HOME`). Sur Ubuntu :

```bash
sudo pip install --break-system-packages torch
```

## Découvrir les GPU

```python
import partagpu

gpus = partagpu.discover()
# Une entree par CUDA device. Un PC avec 2 GPU produit 2 entrees,
# meme `host` / `ip`, `device_index` distinct.
# [GPU('local',  ip='192.168.70.103', dev=0, limit=100%, verified),
#  GPU('local',  ip='192.168.70.103', dev=1, limit=100%, verified),
#  GPU('César 2', ip='192.168.70.105', dev=0, limit=50%,  verified)]
```

## Exécuter une commande sur un pair (`run_remote`)

```python
import partagpu

peer = next(g for g in partagpu.discover() if g.host != "local")

result = partagpu.run_remote(
    peer,
    ["python3", "-c", "import torch; print(torch.cuda.get_device_name(0))"],
    timeout=30,
)
print(result.stdout)
result.check()  # raise si exit != 0
```

`run_remote` accepte aussi :
- `network=True` — laisse le sandbox accéder au réseau (requis pour DDP)
- `workspace={path: content}` ou `workspace=[Path, ...]` — pousse des fichiers dans le `/workspace` du sandbox avant exécution
- `timeout=int` — secondes (défaut 300)
- `user="alice"` — label informatif côté pair

## Entraînement distribué (`distribute`)

```python
import partagpu

results = partagpu.distribute(
    "train.py",
    args=["--epochs", "10"],
    extra_files=["config.yaml", "utils.py"],
    timeout=1800,
)
for r in results:
    print(r.target_machine, r.exit_code)
    print(r.stdout[-500:])
```

`distribute` :
- découvre tous les GPU de la salle (sauf si `gpus=` est passé). **Multi-GPU par machine** géré : un PC avec 4 GPU contribue 4 workers ;
- pousse `train.py` (et `extra_files`) dans le sandbox de chaque pair ;
- définit `MASTER_ADDR`, `MASTER_PORT`, `RANK`, `WORLD_SIZE`, `LOCAL_RANK`, `CUDA_VISIBLE_DEVICES`, `PARTAGPU_LOCAL_RANK`, `BACKEND` sur chaque worker ;
- isole chaque worker à un seul GPU physique via `CUDA_VISIBLE_DEVICES` (le script utilise toujours `cuda:0`, peu importe l'index physique) ;
- ouvre l'isolation réseau du sandbox de chaque pair (NCCL/Gloo rendezvous) ;
- lance les `world_size` workers en parallèle (sur les machines respectives), attend tous les résultats.

Côté `train.py`, vous initialisez DDP standard :

```python
import os
import torch.distributed as dist

dist.init_process_group(backend=os.environ["BACKEND"], init_method="env://")
rank = int(os.environ["RANK"])
world_size = int(os.environ["WORLD_SIZE"])
# ... votre training ...
dist.destroy_process_group()
```

## Configurer le backend manuellement (`setup_ddp` / `cleanup_ddp`)

Si vous voulez orchestrer DDP par vous-même (autre que `distribute`), les helpers minimaux sont disponibles :

```python
from partagpu.distributed import setup_ddp, cleanup_ddp

setup_ddp(rank=0, world_size=2, master_addr="192.168.70.103", backend="nccl")
# ... votre code DDP ...
cleanup_ddp()
```

## Voir aussi

- [README principal](https://github.com/cesar-lizurey/partagpu) — installation de l'app, gestion de salle, UI
- [Guide Python complet](../docs/PYTHON.md) — diagnostic, exemples détaillés, référence des paramètres
- [Architecture](../docs/ARCHITECTURE.md) — comment fonctionne le dispatch pair-à-pair, le sandbox, DDP
- Notebook d'exemples : `examples/decouverte_gpu.ipynb`
- Smoke tests : `examples/smoke_run_remote.py`, `examples/smoke_ddp.py`, `examples/smoke_multi_gpu.py`
