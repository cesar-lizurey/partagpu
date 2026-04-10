#!/bin/bash
# Build and install the PartaGPU privilege helper binary and PolicyKit policy.
# Run this once after cloning, or during packaging.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="${SCRIPT_DIR}/.."
HELPER_CRATE="${PROJECT_DIR}/src-tauri/helper"
POLICY_SRC="${PROJECT_DIR}/src-tauri/resources/com.partagpu.policy"

HELPER_DEST="/usr/local/lib/partagpu/partagpu-helper"
POLICY_DEST="/usr/share/polkit-1/actions/com.partagpu.policy"

if [ "$EUID" -ne 0 ]; then
    echo "Ce script doit être lancé avec sudo :"
    echo "  sudo bash $0"
    exit 1
fi

echo "Compilation du helper Rust..."
# Build as the original user (not root) to avoid permission issues with cargo cache
ORIGINAL_USER="${SUDO_USER:-$(logname 2>/dev/null || echo root)}"
su - "$ORIGINAL_USER" -c "cd '${HELPER_CRATE}' && cargo build --release" 2>&1

HELPER_BIN="${PROJECT_DIR}/src-tauri/target/release/partagpu-helper"
if [ ! -f "$HELPER_BIN" ]; then
    echo "Erreur: le binaire n'a pas été trouvé à ${HELPER_BIN}"
    exit 1
fi

echo "Installation du helper..."
mkdir -p "$(dirname "$HELPER_DEST")"
cp "$HELPER_BIN" "$HELPER_DEST"
chmod 755 "$HELPER_DEST"
chown root:root "$HELPER_DEST"
echo "  -> $HELPER_DEST"

echo "Installation de la policy PolicyKit..."
mkdir -p "$(dirname "$POLICY_DEST")"
cp "$POLICY_SRC" "$POLICY_DEST"
chmod 644 "$POLICY_DEST"
chown root:root "$POLICY_DEST"
echo "  -> $POLICY_DEST"

echo ""
echo "Installation terminée."
