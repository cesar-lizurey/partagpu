# partagpu

Client Python pour [PartaGPU](https://github.com/cesar-lizurey/partagpu) — utilisez les GPU de plusieurs machines d'une salle de cours pour l'entraînement distribué.

## Installation

```bash
pip install partagpu
```

## Utilisation

L'application PartaGPU doit tourner sur votre machine.

### Lister les GPU disponibles

```python
import partagpu

gpus = partagpu.discover()
# [GPU('local', ip='192.168.70.103', limit=100%, verified),
#  GPU('César 2', ip='192.168.70.105', limit=50%, verified)]
```

### Lancer un entraînement distribué

```python
from partagpu.distributed import launch_workers

workers = launch_workers("train.py", args=["--epochs", "10"])
for w in workers:
    w.wait()
```

Voir le [README principal](https://github.com/cesar-lizurey/partagpu#package-python--entraînement-distribué) pour la documentation complète.
