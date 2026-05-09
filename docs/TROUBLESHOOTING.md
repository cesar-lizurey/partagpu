🇬🇧 [English version](TROUBLESHOOTING.en.md)

# Diagnostic — Que faire si ça ne marche pas

Liste des erreurs courantes rencontrées en utilisant l'application **et** le package Python `partagpu`, avec leur cause et leur solution. Pour les bases d'utilisation, voir le [README](../README.md). Pour le détail technique, voir [ARCHITECTURE.md](ARCHITECTURE.md).

---

## Côté application

### L'application ne démarre pas, ou plante au lancement

```bash
# Lancer en console pour voir les logs
/usr/bin/partagpu          # version installée
# ou
npm run tauri:dev          # version de développement
```

Les causes typiques peuvent être :

- **l'absence de Wayland ou X** : un environnement graphique est nécessaire pour la GUI Tauri ;
- **un paquet Tauri ou WebKit manquant** sur le système : `sudo apt install libwebkit2gtk-4.1-dev`.

### Le partage ne s'active pas

L'interface demande le mot de passe (pkexec) puis l'opération échoue. Les causes peuvent être :

- **PolicyKit n'est pas installé** : `sudo apt install policykit-1` ;
- **le helper n'est pas installé** : relancer `sudo bash scripts/install-helper.sh` ;
- **le mot de passe utilisateur est incorrect** : c'est le mot de passe **administrateur** de la machine qui est demandé, pas celui du compte `partagpu`.

### Bandeau « Failed to initialize NVML: Driver/Library version mismatch »

Le module noyau NVIDIA chargé est d'une version différente des bibliothèques en espace utilisateur (en général après un `apt upgrade` non suivi d'un redémarrage).

```bash
# Vérifier les versions en présence
cat /proc/driver/nvidia/version                     # version du module
ls -l /usr/lib/x86_64-linux-gnu/libnvidia-ml.so*    # version des bibliothèques
dpkg -l | grep nvidia-driver                        # version du paquet

# Solution : redémarrer
sudo reboot
```

### Jauge GPU : « indicative (CUDA MPS inactif, la limite n'est pas appliquée) »

L'avertissement signifie que le daemon **CUDA MPS** (*Multi-Process Service*) n'est pas disponible sur la machine. Les limites CPU et RAM continuent d'être appliquées par les cgroups v2, mais la limite GPU n'est plus qu'**indicative** : elle est annoncée aux pairs mais aucune contrainte n'est appliquée côté CUDA, et une tâche peut saturer le GPU à 100 %.

PartaGPU démarre le daemon MPS automatiquement à l'activation du partage (commande `setup-mps` du helper), **uniquement si** le binaire `nvidia-cuda-mps-control` est installé. Sinon, le helper journalise un avertissement et poursuit en mode purement indicatif.

```bash
# Vérifier la présence du binaire
which nvidia-cuda-mps-control       # doit renvoyer /usr/bin/nvidia-cuda-mps-control

# Vérifier que le daemon tourne (après activation du partage)
pgrep -laf nvidia-cuda-mps          # doit lister "nvidia-cuda-mps-control -d"

# Logs du daemon (en root)
sudo cat /var/lib/partagpu/mps-log/control.log
sudo cat /var/lib/partagpu/mps-log/server.log
```

**Solution — installer MPS** :

```bash
# Ubuntu / Debian (paquet officiel d'environ 2 à 3 Go, qui fournit aussi nvcc et les bibliothèques CUDA)
sudo apt install nvidia-cuda-toolkit
```

Puis, dans l'application : **Mon partage → Désactiver → Activer**. Sans ce cycle, le helper ne (re)lance pas MPS et l'avertissement persiste.

**Si `server.log` reste vide alors que le daemon tourne et que des tâches GPU s'exécutent**, c'est que les tâches dans le bac à sable ne se connectent pas à la socket MPS — généralement parce que le montage par *bind* `/var/lib/partagpu/mps` ou la variable `CUDA_MPS_PIPE_DIRECTORY` ne sont pas propagés dans l'environnement de la tâche. Lancer une tâche `printenv | grep MPS` depuis *Mon utilisation* permet de le vérifier.

---

## Pairs et découverte

### Un pair n'apparaît pas dans la liste

Il faut vérifier, dans l'ordre :

1. **les deux machines ont bien l'application lancée** (`ps -ef | grep partagpu`) ;
2. **le pair est sur le même sous-réseau** (mêmes 3 premiers octets d'IP en général). PartaGPU ne traverse pas de NAT ;
3. **le pare-feu autorise l'UDP 5353 (mDNS)** :
   ```bash
   sudo ufw status | grep 5353
   ```
4. **Avahi tourne** (le daemon mDNS système, dont certaines configurations ont besoin) : `sudo systemctl status avahi-daemon`.

### Le pair apparaît mais est marqué « non vérifié »

Le défi HMAC actif sur `/peer/v1/verify` n'a pas répondu correctement. Les causes peuvent être :

- **les pairs ne sont pas dans la même salle** : passphrases différentes, donc `auth_key` différentes, et le HMAC du défi ne correspond pas. Il faut *Quitter la salle* sur l'un et la rejoindre avec le bon code ;
- **les versions de PartaGPU sont incohérentes** : tous les pairs d'une salle doivent exécuter la même version majeure de PartaGPU. Vérifier le badge de version dans l'en-tête de l'application sur chaque machine ;
- **le sondage a expiré (3 s)** : le pare-feu bloque le port 7655, le pair est trop loin sur le LAN, ou son application est encore en train de démarrer. La revérification automatique a lieu toutes les 60 s ; il suffit d'attendre une minute.

Concernant le décalage d'horloge : il n'affecte pas la vérification `/verify` (qui s'appuie sur un *nonce*, pas sur une fenêtre temporelle), mais il bloque les envois de tâches (l'en-tête HTTP `X-PartaGPU-AUTH` impose une fenêtre anti-rejeu de ±30 s). Il reste recommandé d'activer NTP partout :

```bash
sudo timedatectl set-ntp true
timedatectl status      # vérifier la ligne « System clock synchronized: yes »
```

### Plusieurs pairs avec le même nom d'hôte (badge « Conflit »)

Deux machines annoncent le même nom d'hôte (`uname -n`). Cela ne présente aucun risque pour le fonctionnement (PartaGPU différencie par IP), mais l'interface affiche un avertissement. Pour le faire taire :

```bash
sudo hostnamectl set-hostname pc-salle-104    # nouveau nom d'hôte
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

- si `/api/status` ne répond pas, l'application n'a pas démarré son serveur HTTP. Plantage au démarrage ? Consulter les logs (`npm run tauri:dev`).
- si `/api/peers` est vide, aucun pair n'a été découvert. Voir la section *Pairs et découverte* ci-dessus.
- si `/api/gpu` ne contient aucun GPU local, c'est que `nvidia-smi` ne fonctionne pas (incompatibilité entre le pilote et la bibliothèque, voir plus haut).
- si les pairs sont présents mais qu'aucun n'apparaît dans `/api/gpu`, c'est qu'ils ne partagent pas (`sharing_enabled=false`) ou qu'ils ne sont pas vérifiés (`verified=false`). Il faut leur demander.

---

## `run_remote` et `distribute`

### `RemoteTaskError: Dispatch refusé (HTTP 412) : ... salle PartaGPU`

Vous n'êtes dans aucune salle. Dans l'interface : onglet en haut → *Créer une salle* ou *Rejoindre une salle*.

### `RemoteTaskError: Le pair ... a refusé la tâche (HTTP 401) : auth invalide`

Soit la salle ne correspond pas (mauvaise `auth_key`), soit le décalage d'horloge entre les deux PC dépasse 30 s, soit l'en-tête a été altéré en transit. Voir *Pairs et découverte* ; le journal de sécurité détaille la cause exacte.

### `HTTP 415 Unsupported Media Type` côté pair

Tous les corps de requête échangés entre pairs sont chiffrés (AES-256-GCM). Le pair récepteur retourne un code 415 si :

- le client envoie le corps en clair — il faut vérifier les versions de PartaGPU sur les deux machines ;
- le `Content-Type` n'est pas `application/x-partagpu-encrypted-v1` ;
- le pair est dans une autre salle (que le HMAC de l'en-tête passe par hasard avec la mauvaise clé est quasi impossible ; mais comme l'`auth_key` est dérivée du même secret que la `room_key`, en pratique une `auth_key` incohérente provoque un 401 avant même d'atteindre le 415).

Pour vérifier que les deux pairs partagent bien le même secret : sur chaque machine, *Mon partage* → *Salle* doit afficher le même nom et la même passphrase.

### `Commande refusée : « X » n'est pas dans la liste autorisée`

La liste d'autorisation (*allowlist*) du pair ne contient pas la commande. Sur la machine du pair, dans l'interface : *Mon partage* → *Allowlist* → ajouter le binaire. La liste par défaut comprend `python3`, `python`, `nvidia-smi`, `bash`, `make`, `gcc`, `julia`, etc.

### Le bac à sable plante avec `Permission non accordée (os error 13)`

La version de PartaGPU côté pair est obsolète : `git pull && npm run tauri:dev` sur le pair pour la mettre à jour. Le *workspace* doit être créé sous `/tmp` (où l'application peut écrire), pas sous `/var/lib/partagpu` (mode 700, propriétaire `partagpu`).

### `bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted`

Le noyau Ubuntu 24.04+ restreint la création de *user namespaces* non privilégiés (`kernel.apparmor_restrict_unprivileged_userns=1`). *bubblewrap* ne peut donc pas configurer l'interface `lo` dans son espace de noms réseau, et chaque tâche reçue échoue avec ce message.

Le `.deb` PartaGPU dépose normalement un fichier de configuration sysctl additionnel `/etc/sysctl.d/60-partagpu-userns.conf` qui repasse ce réglage à `0` au moment de l'installation. Si l'erreur survient, c'est qu'il a été supprimé, ou que l'installation s'est faite via AppImage ou depuis les sources. Pour le réappliquer :

```bash
sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
echo 'kernel.apparmor_restrict_unprivileged_userns=0' | sudo tee /etc/sysctl.d/60-partagpu-userns.conf
```

Le réglage persiste après redémarrage grâce au fichier déposé dans `/etc/sysctl.d/`. Pour revenir au comportement par défaut d'Ubuntu 24+ : `sudo rm /etc/sysctl.d/60-partagpu-userns.conf && sudo sysctl --system`.

### Une tâche reste en `Queued` (en file d'attente) indéfiniment

Le bac à sable n'arrive pas à se lancer. Les causes peuvent être :

- ***bubblewrap* n'est pas installé sur la machine cible** : `sudo apt install -y bubblewrap` ;
- **le compte `partagpu` n'a pas été créé** : sur la machine cible, dans l'interface, *Mon partage* → *Désactiver* puis *Activer* (cette manipulation relance la création de l'utilisateur via le helper).

### `ModuleNotFoundError: No module named 'torch'` côté pair

Le bac à sable tourne sous l'UID `partagpu`, qui ne voit pas le venv utilisateur. Il existe deux solutions :

**Solution recommandée — via l'interface (venv géré)** : sur chaque machine cible, *Mon partage* → *Environnement Python pour les tâches reçues* → cliquer sur **Installer la toolkit ML (~3 Go)**. Le mot de passe administrateur est demandé une fois, et le téléchargement prend 5 à 10 minutes. Cette opération installe `torch`, `torchvision`, `numpy`, `scipy`, `pandas`, `scikit-learn`, `matplotlib` et `pillow`. Le bac à sable monte ensuite `/var/lib/partagpu/venv/` automatiquement et fait pointer `python3` dessus. Aucune pollution du Python système.

**Solution alternative — installation système** :

```bash
sudo apt install -y python3-pip
sudo /usr/bin/python3 -m pip install --break-system-packages \
  torch torchvision numpy scipy pandas scikit-learn matplotlib pillow
```

À effectuer sur **chaque machine** qui doit recevoir des tâches PyTorch.

**Pour ajouter un paquet au venv géré** (par exemple `transformers` ou `jax`) :

```bash
sudo /var/lib/partagpu/venv/bin/pip install transformers
```

Aucune interface dédiée n'existe encore pour cela ; l'installation se fait directement avec le `pip` du venv, à condition d'avoir d'abord cliqué sur *Installer la toolkit ML*.

### `Failed to initialize NumPy: No module named 'numpy'` (avertissement au démarrage de torch)

Le message est bénin (torch fonctionne quand même) mais désagréable. La solution consiste à installer NumPy dans le Python système, comme indiqué ci-dessus.

### NCCL bloque lors de l'`init_process_group`

Le port 29500 (rendez-vous NCCL) n'est pas joignable entre les machines. Pour le tester depuis la machine A :

```bash
nc -zv 192.168.x.y 29500     # IP de la machine B
```

Si la connexion est refusée ou si elle expire, les causes peuvent être :

- **le pare-feu n'est pas ouvert sur la cible** : vérifier avec `sudo ufw status | grep 29500`. Si rien n'apparaît, désactiver puis réactiver le partage (ce qui relance le helper et ouvre le port). On peut aussi ouvrir directement la plage : `sudo ufw allow 29500:29510/tcp` ;
- **le helper n'est pas à jour** sur la cible : `git pull && npm run helper:build && sudo bash scripts/install-helper.sh && npm run tauri:dev`.

### `CUDA error: invalid device ordinal`

Le script utilise `cuda:1` (ou supérieur) alors que `CUDA_VISIBLE_DEVICES` filtre déjà à un seul GPU. Toujours utiliser `cuda:0` ou `cuda:LOCAL_RANK` (qui vaut 0) dans un script lancé par `partagpu.distribute`.

### `distribute()` lève `RemoteTaskError: Rank N a échoué avant de produire un résultat`

Une exception est levée côté Python avant même que la tâche ne soit envoyée (*dispatch*). Il faut lire le message complet ; les causes habituelles sont :

- une dépendance Python manquante sur **votre** machine (pas chez le pair) ;
- un fichier référencé dans `extra_files` qui n'existe pas ;
- un argument mal typé passé à `partagpu.distribute(...)`.

### Sorties tronquées

La sortie standard est plafonnée à **1 Mo** par tâche, et la sortie d'erreur à **256 Ko**. Pour des sorties volumineuses, il vaut mieux écrire dans un fichier (et envisager de le faire remonter via un système de fichiers partagé ou un téléversement séparé — non géré par PartaGPU pour l'instant).

### Mes `print()` n'apparaissent qu'à la fin (pas en direct)

Python met sa sortie standard en mémoire tampon par bloc quand il n'écrit pas vers un terminal (TTY) — c'est notre cas, puisqu'il écrit dans un *pipe* vers le bac à sable. Il existe trois façons de forcer un envoi ligne par ligne :

```python
print("hello", flush=True)            # appel explicite à chaque ligne
```

```bash
python3 -u mon_script.py               # mode sans tampon global
```

```python
import os
os.environ.setdefault("PYTHONUNBUFFERED", "1")  # à mettre au début du script
```

Le symptôme classique est de voir le panneau de sortie en direct rester vide pendant 30 s, puis tout sortir d'un coup à la fin. La cause est presque toujours la mise en mémoire tampon de Python, pas un bug d'infrastructure.

`tqdm` et les appels `print('\\r…', end='')` (barres de progression) écrivent sans saut de ligne et sont mis en tampon différemment — passer `tqdm(file=sys.stderr)` règle souvent le problème.

---

## Tests rapides (*smoke tests*)

Pour vérifier l'installation à chaque étape :

```bash
cd examples
source venv/bin/activate     # ou utiliser ./venv/bin/python directement
```

| Script | Ce qu'il valide | Quand le lancer |
|---|---|---|
| `smoke_run_remote.py` | L'envoi d'une commande exécutée dans le bac à sable, en boucle locale. | Après la première installation, pour vérifier que l'application et le bac à sable fonctionnent. |
| `smoke_ddp.py` | Un DDP en boucle locale avec `world_size=1`, puis en multi-machines si `PARTAGPU_TEST_MULTI=1`. | Avant de lancer un vrai entraînement DDP. |
| `smoke_multi_gpu.py` | La logique multi-GPU (avec `PARTAGPU_FORCE_GPU_COUNT=2` au lancement de l'application). | Pour vérifier l'envoi sur des serveurs multi-GPU. |

Si l'un d'eux échoue, la cause est généralement décrite dans l'une des sections ci-dessus.

---

## Annuler une tâche en cours

Il existe trois façons d'annuler une tâche, par ordre d'usage habituel :

1. **Le bouton Stop dans l'interface** : sur chaque ligne du tableau *Mes tâches en cours* (tâches sortantes) ou *Qui utilise mes ressources ?* (tâches entrantes), un bouton **Stop** apparaît tant que la tâche est `Queued` (en file d'attente) ou `Running` (en cours). Un clic provoque l'annulation immédiate, propagée au pair (SIGTERM côté bac à sable, puis SIGKILL après 2 s si la tâche ne répond pas).

2. **Un `Ctrl+C` dans un notebook** : `partagpu.run_remote(...)` et `partagpu.distribute(...)` interceptent l'exception `KeyboardInterrupt`, envoient une requête `POST /api/cancel` à l'application locale (qui la propage au pair sous forme de `DELETE`), puis relèvent l'exception. Avec `distribute`, **tous** les rangs sont annulés.

3. **Par programme** : `partagpu.cancel(local_id)`, où `local_id` est l'identifiant retourné dans `TaskResult`. Cette approche est pratique pour annuler depuis un autre notebook ou un script.

```python
import partagpu, threading

def long_task():
    partagpu.run_remote(peer, ["python3", "-c", "import time; time.sleep(3600)"],
                        timeout=3600, local_id="ma-tache-de-test")

t = threading.Thread(target=long_task)
t.start()

# Plus tard, depuis une autre cellule ou un autre script :
partagpu.cancel("ma-tache-de-test")
```

### `distribute()` : annulation automatique en cascade quand un rang plante

Si un rang meurt en cours d'entraînement DDP, les autres restent en théorie bloqués sur `init_process_group` ou sur une opération de réduction collective (*all-reduce*) jusqu'au délai d'expiration NCCL (environ 30 minutes). `distribute()` détecte le premier rang qui échoue et **annule automatiquement** tous les autres dans la foulée, afin qu'aucune machine ne reste à attendre dans le vide.

### Une tâche est marquée `Cancelled` (annulée) dans l'interface mais semble continuer

Le `SIGTERM` peut être ignoré par certains scripts (par exemple un gestionnaire de signal Python qui ne passe pas la main au `kill -9`). Le minuteur de 2 s envoie alors un `SIGKILL`. Si la tâche apparaît encore en cours après 5 s :

- vérifier que le helper est à jour (`npm run helper:build && sudo bash scripts/install-helper.sh`) ;
- consulter les logs côté pair, dans le terminal de `npm run tauri:dev`. Si l'erreur mentionne `kill: pas trouvé` ou un message similaire, le helper n'a pas accès à l'outil `kill` — il faut corriger le `PATH` du compte `partagpu` ou installer `procps`.

---

## Logs et observabilité

### Logs de l'application

```bash
# Mode développement : tout est affiché dans le terminal de npm run tauri:dev
npm run tauri:dev 2>&1 | tee /tmp/partagpu.log

# Mode production : pas de fichier de log dédié, il faut lancer l'application via un terminal
/usr/bin/partagpu
```

### Journal de sécurité (intégré à l'interface)

Onglet *Mon partage* → *Journal de sécurité*. Il conserve les 500 derniers événements (pairs détectés, tâches acceptées ou refusées, conflits de nom d'hôte, etc.).

### Inspecter l'état d'une tâche en cours

```bash
# Liste des tâches entrantes (côté pair)
curl -s -H "X-PartaGPU-AUTH: 123456" \
    http://127.0.0.1:7655/peer/v1/tasks/<task-id> | python3 -m json.tool
```

L'en-tête doit être calculé avec `compute_request_auth(method, path, body)`. Depuis `curl`, c'est peu commode ; il vaut mieux passer par `cargo run --example sign-request` ou par un court script Python qui calcule le HMAC.

---

## Quand rien de tout cela ne fonctionne

Réinitialisation complète pour repartir d'une base saine :

```bash
# 1. Quitter la salle dans l'interface (cela efface ~/.config/partagpu/room.json)
# 2. Désactiver le partage dans l'interface

# 3. Arrêter l'application
pkill -f /usr/bin/partagpu
pkill -f target/debug/partagpu

# 4. Supprimer l'utilisateur partagpu (retour à l'état initial)
sudo /usr/local/lib/partagpu/partagpu-helper remove-user

# 5. Supprimer la configuration
rm -rf ~/.config/partagpu

# 6. Relancer l'application et refaire la configuration (créer la salle, activer le partage, etc.)
npm run tauri:dev
```

Cette procédure s'applique si vous avez perdu confiance dans l'état du système après une série d'essais infructueux. Il s'agit d'une réinitialisation uniquement côté utilisateur ; elle ne touche pas à l'installation système.
