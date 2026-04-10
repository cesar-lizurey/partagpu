#!/bin/bash
# Remove the PartaGPU privilege helper and PolicyKit policy.

set -euo pipefail

HELPER_DEST="/usr/local/lib/partagpu/partagpu-helper"
POLICY_DEST="/usr/share/polkit-1/actions/com.partagpu.policy"

if [ "$EUID" -ne 0 ]; then
    echo "Ce script doit être lancé avec sudo :"
    echo "  sudo bash $0"
    exit 1
fi

echo "Désinstallation du helper PartaGPU..."

rm -f "$HELPER_DEST"
rmdir --ignore-fail-on-non-empty /usr/local/lib/partagpu 2>/dev/null || true
echo "  Supprimé : $HELPER_DEST"

rm -f "$POLICY_DEST"
echo "  Supprimé : $POLICY_DEST"

echo "Désinstallation terminée."
