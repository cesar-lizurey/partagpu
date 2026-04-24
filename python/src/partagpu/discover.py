"""Discover available GPU resources via the PartaGPU local HTTP API."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional

import requests

API_BASE = "http://127.0.0.1:7654"


@dataclass
class GPUResource:
    """A GPU resource available for distributed training."""

    host: str
    ip: str
    gpu_limit_percent: float
    verified: bool

    def __repr__(self) -> str:
        status = "verified" if self.verified else "unverified"
        return f"GPU({self.host!r}, ip={self.ip!r}, limit={self.gpu_limit_percent}%, {status})"


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


def discover(api_base: str = API_BASE, timeout: float = 2.0) -> list[GPUResource]:
    """Discover all available GPUs (local + remote peers).

    Requires the PartaGPU desktop app to be running.

    Returns:
        List of GPUResource objects representing available GPUs.

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

    return [GPUResource(**gpu) for gpu in resp.json()]


def get_peers(api_base: str = API_BASE, timeout: float = 2.0) -> list[Peer]:
    """Get all peers discovered by PartaGPU.

    Returns:
        List of Peer objects.
    """
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
            )
        )
    return peers
