# Diagnostic — Que faire si ça ne marche pas

Liste des erreurs courantes rencontrées en utilisant l'app **et** le package Python `partagpu`, avec leur cause et leur fix. Pour les bases d'utilisation, voir le [README](../README.md). Pour le détail technique, voir [ARCHITECTURE.md](ARCHITECTURE.md).

---

## Côté application

### L'app ne démarre pas / crash au lancement

```bash
# Lancer en console pour voir les logs
/usr/bin/partagpu          # version installée
# ou
npm run tauri:dev          # version dev
```

Causes typiques :
- **Pas de Wayland/X** : nécessaire pour la GUI Tauri.
- **Tauri/webkit manquant** sur le système : `sudo apt install libwebkit2gtk-4.1-dev`.

### Le partage ne s'active pas

L'UI demande le mot de passe (pkexec) puis échoue. Causes :
- **PolicyKit pas installé** : `sudo apt install policykit-1`.
- **Helper pas installé** : refaire `sudo bash scripts/install-helper.sh`.
- **Mot de passe utilisateur incorrect** : c'est le mot de passe **administrateur** de la machine qui est demandé, pas celui du compte `partagpu`.

### Bandeau "Failed to initialize NVML: Driver/Library version mismatch"

Le module noyau NVIDIA chargé est d'une version différente des libs userland (typiquement après un `apt upgrade` non suivi d'un reboot).

```bash
# Verifier
cat /proc/driver/nvidia/version           # version du module
ls -l /usr/lib/x86_64-linux-gnu/libnvidia-ml.so*    # version des libs
dpkg -l | grep nvidia-driver              # version du paquet

# Fix : redemarrer
sudo reboot
```

---

## Pairs et découverte

### Un pair n'apparaît pas dans la liste

Vérifier dans l'ordre :
1. **Les deux machines ont l'app lancée** (`ps -ef | grep partagpu`).
2. **Le pair est sur le même sous-réseau** (mêmes 3 premiers octets d'IP en général). PartaGPU n'a pas de NAT traversal.
3. **Le firewall autorise UDP 5353 (mDNS)** :
   ```bash
   sudo ufw status | grep 5353
   ```
4. **Avahi tourne** (mDNS daemon système, certains setups en ont besoin) : `sudo systemctl status avahi-daemon`.

### Le pair apparaît mais marqué `non vérifié`

Le code TOTP ne match pas. Causes :
- **Pas dans la même salle** : différentes passphrases. *Quitter la salle* sur l'un, la rejoindre avec le bon code.
- **Décalage d'horloge > 30 s entre les machines** : les codes TOTP sont valides ±30 s seulement. Activer NTP partout :
  ```bash
  sudo timedatectl set-ntp true
  timedatectl status      # verifier System clock synchronized: yes
  ```

### Plusieurs pairs avec le même hostname (badge "Conflit")

Deux machines annoncent le même hostname (`uname -n`). Aucun risque pour le fonctionnement (PartaGPU différencie par IP), mais l'UI affiche un avertissement. Pour le faire taire :
```bash
sudo hostnamectl set-hostname pc-salle-104    # nouveau hostname
sudo reboot
```

---

## API HTTP locale

### `partagpu.discover()` retourne 0 GPU

```bash
# 1. App joignable ?
curl -s http://127.0.0.1:7654/api/status

# 2. mDNS voit-il les pairs ?
curl -s http://127.0.0.1:7654/api/peers | python3 -m json.tool

# 3. L'app voit-elle le GPU local ?
curl -s http://127.0.0.1:7654/api/gpu | python3 -m json.tool
```

Selon ce qui manque :
- `/api/status` ne répond pas → l'app n'a pas démarré son serveur HTTP. Crash au démarrage ? Voir les logs (`npm run tauri:dev`).
- `/api/peers` est vide → aucun pair découvert. Voir la section *Pairs et découverte* ci-dessus.
- `/api/gpu` ne contient aucun GPU local → `nvidia-smi` ne marche pas (driver/lib mismatch — voir ci-dessus).
- Les pairs sont là mais aucun n'apparaît dans `/api/gpu` → ils ne partagent pas (`sharing_enabled=false`) ou ne sont pas vérifiés (`verified=false`). Le leur demander.

---

## `run_remote` et `distribute`

### `RemoteTaskError: Dispatch refusé (HTTP 412) : ... salle PartaGPU`

Vous n'êtes dans aucune salle. UI → onglet en haut → *Créer une salle* ou *Rejoindre une salle*.

### `RemoteTaskError: Le pair ... a refusé la tâche (HTTP 401) : Code TOTP invalide`

Décalage d'horloge entre les deux PC ou pas dans la même salle. Voir *Pairs et découverte*.

### `Commande refusée : « X » n'est pas dans la liste autorisée`

L'allowlist du pair ne contient pas la commande. Sur la machine du pair : UI → *Mon partage* → *Allowlist* → ajouter le binaire. Defaults : `python3`, `python`, `nvidia-smi`, `bash`, `make`, `gcc`, `julia`, etc.

### Le sandbox plante avec `Permission non accordée (os error 13)`

Bug fixé en `1.1.0+`. `git pull && npm run tauri:dev` côté pair pour mettre à jour. Cause : ancienne version créait le workspace dans `/var/lib/partagpu` (mode 700, owned par partagpu) au lieu de `/tmp` writable par l'app.

### Une tâche reste en `Queued` indéfiniment

Le sandbox n'arrive pas à se lancer. Causes :
- **`bubblewrap` pas installé sur la machine cible** : `sudo apt install -y bubblewrap`.
- **Le compte `partagpu` n'a pas été créé** : sur la machine cible, UI → *Mon partage* → *Désactiver* puis *Activer* (ça relance la création du user via le helper).

### `ModuleNotFoundError: No module named 'torch'` côté pair

Le sandbox tourne sous l'UID `partagpu` qui ne voit pas votre venv utilisateur. Il faut `torch` dans le **Python système** :
```bash
sudo apt install -y python3-pip
sudo /usr/bin/python3 -m pip install --break-system-packages torch numpy
```

À faire sur **chaque machine** qui doit recevoir des tâches PyTorch.

### `Failed to initialize NumPy: No module named 'numpy'` (warning au démarrage de torch)

Bénin (torch fonctionne quand même) mais désagréable. Fix : installer numpy en system Python comme ci-dessus.

### NCCL hang au `init_process_group`

Le port 29500 (rendezvous) n'est pas joignable entre les machines. Tester depuis la machine A :
```bash
nc -zv 192.168.x.y 29500     # IP de la machine B
```

Si refusé/timeout :
- **Firewall pas ouvert sur la cible**. Vérifier : `sudo ufw status | grep 29500`. Si rien : refaire un toggle off/on du partage (ça relance le helper qui ouvre le port). Ou directement : `sudo ufw allow 29500:29510/tcp`.
- **Helper pas à jour** sur la cible. `git pull && npm run helper:build && sudo bash scripts/install-helper.sh && npm run tauri:dev`.

### `CUDA error: invalid device ordinal`

Le script utilise `cuda:1` (ou supérieur) alors que `CUDA_VISIBLE_DEVICES` filtre déjà à un seul GPU. Toujours utiliser `cuda:0` ou `cuda:LOCAL_RANK` (qui vaut 0) dans un script lancé par `partagpu.distribute`.

### `distribute()` lève `RemoteTaskError: Rank N a échoué avant de produire un résultat`

Une `Exception` est levée du côté Python avant même que la tâche ne soit dispatchée. Lire le message complet — typiquement :
- Une dépendance Python manquante côté **votre** machine (pas le pair).
- Un fichier `extra_files` qui n'existe pas.
- Un argument mal typé passé à `partagpu.distribute(...)`.

### Outputs tronqués

stdout est cappé à **1 Mo**, stderr à **256 Ko** par tâche. Pour des sorties volumineuses, écrire dans un fichier (et envisager de le faire remonter via un shared filesystem ou un upload séparé — pas géré par PartaGPU pour l'instant).

---

## Smoke tests

Pour vérifier l'installation à chaque étape :

```bash
cd examples
source venv/bin/activate     # ou utiliser ./venv/bin/python directement
```

| Script | Ce qu'il valide | Quand le lancer |
|---|---|---|
| `smoke_run_remote.py` | Dispatch d'une commande sandboxée en loopback | Après le premier setup, pour vérifier que l'app + le sandbox fonctionnent |
| `smoke_ddp.py` | DDP loopback world_size=1, puis multi-machine si `PARTAGPU_TEST_MULTI=1` | Avant de lancer un vrai entraînement DDP |
| `smoke_multi_gpu.py` | Logique multi-GPU (avec `PARTAGPU_FORCE_GPU_COUNT=2` au lancement de l'app) | Pour vérifier le dispatch sur des serveurs multi-GPU |

Si l'un échoue, la cause est généralement listée dans une des sections ci-dessus.

---

## Logs et observabilité

### Logs de l'app

```bash
# Mode dev : tout est dans le terminal de npm run tauri:dev
npm run tauri:dev 2>&1 | tee /tmp/partagpu.log

# Mode production : pas de fichier de log dédié, lancer via terminal
/usr/bin/partagpu
```

### Journal de sécurité (intégré à l'UI)

Onglet *Mon partage* → *Journal de sécurité*. Garde les 500 derniers événements (pairs détectés, tâches acceptées/refusées, conflits hostname, etc.).

### Inspecter l'état d'une tâche en cours

```bash
# Liste des taches entrantes (cote pair)
curl -s -H "X-PartaGPU-TOTP: 123456" \
    http://127.0.0.1:7655/peer/v1/tasks/<task-id> | python3 -m json.tool
```

(Code TOTP courant visible dans l'UI sous le nom de la salle.)

---

## Quand rien de tout ça ne marche

Reset complet pour repartir d'une base propre :

```bash
# 1. Quitter la salle dans l'UI (effacer ~/.config/partagpu/room.json)
# 2. Désactiver le partage dans l'UI

# 3. Stopper l'app
pkill -f /usr/bin/partagpu
pkill -f target/debug/partagpu

# 4. Supprimer le user partagpu (revient a l'etat initial)
sudo /usr/local/lib/partagpu/partagpu-helper remove-user

# 5. Supprimer la config
rm -rf ~/.config/partagpu

# 6. Relancer l'app, refaire la config (creer salle, activer partage, etc.)
npm run tauri:dev
```

À faire si vous avez perdu confiance dans l'état du système après une série d'essais foireux. Reset utilisateur uniquement, ne touche pas à l'install système.
