"""Discover available GPU resources via the PartaGPU local HTTP API."""

from __future__ import annotations

from dataclasses import dataclass

import requests

API_BASE = "http://127.0.0.1:7654"


@dataclass
class GPUResource:
    """A single CUDA device available for distributed training.

    A peer with N visible GPUs produces N ``GPUResource`` instances — same
    ``host`` / ``ip`` but distinct ``device_index`` (0, 1, ..., N-1). Pass
    one of these to :func:`partagpu.run_remote` to target that exact peer
    (the ``device_index`` is informational at the run_remote level — it's
    used by :func:`partagpu.distribute` to set ``CUDA_VISIBLE_DEVICES``).
    """

    host: str
    ip: str
    gpu_limit_percent: float
    verified: bool
    device_index: int = 0

    def __repr__(self) -> str:
        status = "verified" if self.verified else "unverified"
        return (
            f"GPU({self.host!r}, ip={self.ip!r}, dev={self.device_index}, "
            f"limit={self.gpu_limit_percent}%, {status})"
        )


@dataclass
class Peer:
    """A machine discovered on the network by PartaGPU."""

    display_name: str
    hostname: str
    ip: str
    sharing_enabled: bool
    cpu_limit: float
    ram_limit: float
    gpu_limit: float
    verified: bool
    gpu_count: int = 0


def discover(api_base: str = API_BASE, timeout: float = 2.0) -> list[GPUResource]:
    """Discover all available GPUs (local + remote peers).

    Requires the PartaGPU desktop app to be running.

    Returns:
        List of :class:`GPUResource` — **one entry per CUDA device**. A peer
        with 4 GPUs produces 4 entries with ``device_index`` 0..3.

    Raises:
        ConnectionError: If the PartaGPU app is not running.
    """
    try:
        resp = requests.get(f"{api_base}/api/gpu", timeout=timeout)
        resp.raise_for_status()
    except requests.ConnectionError:
        raise ConnectionError(
            "Impossible de se connecter a PartaGPU. "
            "Verifiez que l'application est lancee."
        ) from None
    except requests.RequestException as e:
        raise ConnectionError(f"Erreur API PartaGPU: {e}") from None

    out: list[GPUResource] = []
    for gpu in resp.json():
        out.append(
            GPUResource(
                host=gpu.get("host", ""),
                ip=gpu.get("ip", ""),
                gpu_limit_percent=float(gpu.get("gpu_limit_percent", 0.0)),
                verified=bool(gpu.get("verified", False)),
                device_index=int(gpu.get("device_index", 0)),
            )
        )
    return out


def get_peers(api_base: str = API_BASE, timeout: float = 2.0) -> list[Peer]:
    """Get all peers discovered by PartaGPU."""
    try:
        resp = requests.get(f"{api_base}/api/peers", timeout=timeout)
        resp.raise_for_status()
    except requests.ConnectionError:
        raise ConnectionError(
            "Impossible de se connecter a PartaGPU. "
            "Verifiez que l'application est lancee."
        ) from None

    peers = []
    for p in resp.json():
        peers.append(
            Peer(
                display_name=p.get("display_name", ""),
                hostname=p.get("hostname", ""),
                ip=p.get("ip", ""),
                sharing_enabled=p.get("sharing_enabled", False),
                cpu_limit=p.get("cpu_limit", 0),
                ram_limit=p.get("ram_limit", 0),
                gpu_limit=p.get("gpu_limit", 0),
                verified=p.get("verified", False),
                gpu_count=int(p.get("gpu_count", 0)),
            )
        )
    return peers
