"""PartaGPU — Client Python pour l'entraînement distribué multi-GPU sur réseau local."""

from partagpu.discover import discover, GPUResource

__version__ = "1.0.0"
__all__ = ["discover", "GPUResource"]
