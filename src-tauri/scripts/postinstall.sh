#!/bin/bash
set -e

# ── Helper privilégié + règle PolicyKit ───────────────────────────────────
mkdir -p /usr/local/lib/partagpu
cp /usr/lib/partagpu/resources/partagpu-helper /usr/local/lib/partagpu/partagpu-helper
chmod 755 /usr/local/lib/partagpu/partagpu-helper
chown root:root /usr/local/lib/partagpu/partagpu-helper
cp /usr/lib/partagpu/resources/com.partagpu.policy /usr/share/polkit-1/actions/com.partagpu.policy
chmod 644 /usr/share/polkit-1/actions/com.partagpu.policy

# ── Bubblewrap : autoriser la création de user namespaces non-privilégiés ─
# Ubuntu 24.04+ ship `kernel.apparmor_restrict_unprivileged_userns=1` par
# défaut. Avec ce réglage, bwrap ne peut pas configurer son namespace réseau
# (CAP_NET_ADMIN refusé) et chaque tâche échoue avec :
#
#     bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted
#
# On pose un drop-in sysctl pour repasser au comportement Ubuntu ≤ 23.10 /
# Debian 12. Effet : tout ce qui utilise un user namespace non-privilégié
# (flatpak, chromium sandbox, podman, firejail, bwrap) marche à nouveau.
#
# Idempotent : on ne touche rien si le drop-in existe déjà (l'admin l'a
# peut-être ajusté). Pour revenir au défaut Ubuntu 24+ :
#   sudo rm /etc/sysctl.d/60-partagpu-userns.conf
#   sudo sysctl --system
SYSCTL_FILE=/etc/sysctl.d/60-partagpu-userns.conf
if [ ! -f "$SYSCTL_FILE" ]; then
    cat > "$SYSCTL_FILE" <<'EOF'
# Posé par partagpu — requis par bubblewrap (sandbox par tâche).
# Autorise la création de user namespaces non-privilégiés (défaut sur
# Ubuntu ≤ 23.10 et Debian). Sans ça, bwrap échoue à monter `lo` dans
# son namespace réseau ("Failed RTM_NEWADDR: Operation not permitted").
#
# Pour revenir au défaut Ubuntu 24.04+ :
#   sudo rm /etc/sysctl.d/60-partagpu-userns.conf
#   sudo sysctl --system
kernel.apparmor_restrict_unprivileged_userns=0
EOF
    chmod 644 "$SYSCTL_FILE"
    # Application immédiate (sans reboot). `|| true` car certains conteneurs
    # / chroots d'install n'ont pas accès à /proc/sys ; ce n'est pas grave,
    # le réglage prendra effet au prochain boot via /etc/sysctl.d/.
    sysctl --quiet -p "$SYSCTL_FILE" 2>/dev/null || true
fi
