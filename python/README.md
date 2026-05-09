🇬🇧 [English version](README.en.md)

<p align="center">
  <img src="https://raw.githubusercontent.com/cesar-lizurey/partagpu/main/public/favicon.png" alt="PartaGPU" width="320">
</p>

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
- `network=True` — laisse le sandbox accéder au réseau (téléchargement de données, DDP, etc.)
- `workspace={path: content}` ou `workspace=[Path, ...]` — pousse des fichiers dans le `/workspace` du sandbox avant exécution
- `timeout=int` — secondes (défaut 300)
- `user="alice"` — label informatif côté pair
- `local_id="..."` — id pré-alloué pour pouvoir annuler la tâche par programme avant qu'elle ne retourne
- `live=True` — affiche stdout/stderr du pair en streaming (~250 ms de cadence) au lieu de tout retourner à la fin. Pratique pour suivre un entraînement long depuis un notebook.
- `outputs=["model.pt", ...]` — fichiers à rapatrier depuis le `/workspace` du pair après exit. Disponibles dans `result.artifacts: dict[str, bytes]`. Plafond agrégé 256 MiB par tâche.

`Ctrl+C` dans le notebook propage automatiquement le cancel au pair.

## Annuler une tâche

```python
# Par programme, depuis un autre notebook ou cellule
partagpu.cancel(local_id)
```

Le `local_id` vient de `TaskResult.id` (retourné par `run_remote`/`distribute`), ou que vous avez vous-même fixé via le kwarg `local_id=`.

## Entraînement distribué (`distribute`)

```python
import partagpu

results = partagpu.distribute(
    "train.py",
    args=["--epochs", "10"],
    extra_files=["config.yaml", "utils.py"],
    timeout=1800,
    live=True,                    # logs streames pendant le run
    outputs=["model.pt"],         # rapatrie le checkpoint en RAM
    local=False,                  # n'utilise pas la machine locale
)
for r in results:
    print(r.target_machine, r.exit_code)
    print(r.stdout[-500:])

# Le checkpoint est dans results[0].artifacts (rang 0 par convention DDP)
import io, torch
weights = torch.load(io.BytesIO(results[0].artifacts["model.pt"]), map_location="cpu")
```

`distribute` :
- découvre tous les GPU de la salle (sauf si `gpus=` est passé). **Multi-GPU par machine** géré : un PC avec 4 GPU contribue 4 workers ;
- pousse `train.py` (et `extra_files`) dans le sandbox de chaque pair ;
- définit `MASTER_ADDR`, `MASTER_PORT`, `RANK`, `WORLD_SIZE`, `LOCAL_RANK`, `CUDA_VISIBLE_DEVICES`, `PARTAGPU_LOCAL_RANK`, `BACKEND` sur chaque worker ;
- isole chaque worker à un seul GPU physique via `CUDA_VISIBLE_DEVICES` (le script utilise toujours `cuda:0`, peu importe l'index physique) ;
- ouvre l'isolation réseau du sandbox de chaque pair (NCCL/Gloo rendezvous) ;
- lance les `world_size` workers en parallèle (sur les machines respectives), attend tous les résultats.

Paramètres optionnels supplémentaires :
- `live=True` — chaque rang imprime ses logs préfixés `[rankN]` au fur et à mesure (lock partagé pour ne pas mélanger les lignes entre rangs).
- `outputs=["model.pt", ...]` — chemins relatifs à `/workspace` à rapatrier en fin de tâche. Par convention DDP, seul rank 0 sauve un checkpoint, donc seul `results[0].artifacts` est non-vide. Plafond agrégé 256 MiB par tâche.
- `local=False` — exclut la machine locale de l'auto-discover. Utile quand le partage n'est pas activé localement et que vous voulez juste consommer des GPU distants. Sans ce filtre, rank 0 atterrit sur le peer-API local et tombe sur un 403.

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
- [Architecture](https://github.com/cesar-lizurey/partagpu/blob/main/docs/ARCHITECTURE.md) — comment fonctionne le dispatch pair-à-pair, le sandbox, DDP
- [Diagnostic](https://github.com/cesar-lizurey/partagpu/blob/main/docs/TROUBLESHOOTING.md) — erreurs courantes (auth HMAC mismatch, NCCL, sandbox, torch missing, etc.)
- Notebook d'exemples : [examples/decouverte_gpu.ipynb](https://github.com/cesar-lizurey/partagpu/blob/main/examples/decouverte_gpu.ipynb)
- Smoke tests : [smoke_run_remote.py](https://github.com/cesar-lizurey/partagpu/blob/main/examples/smoke_run_remote.py), [smoke_ddp.py](https://github.com/cesar-lizurey/partagpu/blob/main/examples/smoke_ddp.py), [smoke_multi_gpu.py](https://github.com/cesar-lizurey/partagpu/blob/main/examples/smoke_multi_gpu.py)
