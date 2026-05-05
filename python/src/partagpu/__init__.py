"""PartaGPU — Client Python pour l'entraînement distribué multi-GPU sur réseau local."""

from partagpu.discover import GPUResource, discover
from partagpu.distributed import distribute
from partagpu.remote import RemoteTaskError, TaskResult, cancel, run_remote

__version__ = "1.4.0"
__all__ = [
    "GPUResource",
    "RemoteTaskError",
    "TaskResult",
    "cancel",
    "discover",
    "distribute",
    "run_remote",
]
