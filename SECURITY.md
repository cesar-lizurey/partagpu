🇬🇧 [English version](SECURITY.en.md)

# Sécurité de PartaGPU

Ce document détaille les mesures de sécurité implémentées dans PartaGPU. L'application est conçue pour fonctionner dans un environnement de salle de cours où les postes sont sur le même réseau local, avec un niveau de confiance modéré entre les utilisateurs.

---

## Table des matières

- [Vue d'ensemble](#vue-densemble)
- [1. Authentification des pairs par HMAC + timestamp](#1-authentification-des-pairs-par-hmac--timestamp)
- [2. Chiffrement des messages pair-à-pair](#2-chiffrement-des-messages-pair-à-pair)
- [3. Sandbox d'exécution (bubblewrap)](#3-sandbox-dexécution-bubblewrap)
- [4. Durcissement du compte partagpu](#4-durcissement-du-compte-partagpu)
- [5. Gestion automatique du pare-feu](#5-gestion-automatique-du-pare-feu)
- [6. Protection contre le mDNS spoofing / flood](#6-protection-contre-le-mdns-spoofing--flood)
- [7. Élévation de privilèges sécurisée (PolicyKit)](#7-élévation-de-privilèges-sécurisée-policykit)
- [8. Validation des entrées](#8-validation-des-entrées)
- [Signaler une vulnérabilité](#signaler-une-vulnérabilité)

---

## Vue d'ensemble

PartaGPU repose sur plusieurs couches de sécurité complémentaires :

| Couche | Protège contre | Implémentation |
|--------|---------------|----------------|
| **Authentification HMAC** | Pairs non autorisés, imposteurs | `auth_key` dérivée via PBKDF2 600 k iters (slow KDF), challenge actif `/peer/v1/verify` côté discovery, header `X-PartaGPU-AUTH` lié au corps des requêtes |
| **Anti-rejeu** | Replay d'une requête capturée dans la fenêtre 30 s | `ReplayCache` en mémoire qui dédupe les `X-PartaGPU-AUTH` vus |
| **Chiffrement AES-256-GCM** | Écoute réseau passive | Clé HKDF du secret de salle + ECDH X25519 forward-secret, mandatory sur /peer/v1/tasks* |
| **Sandbox bubblewrap** | Exécution de code malveillant | Filesystem read-only, pas de réseau, PID isolé, cgroup CPU/RAM/`pids.max=1024` (anti fork bomb), cap GPU SM via CUDA MPS si dispo |
| **Compte durci** | Abus du compte partagpu | Shell restreint, SSH bloqué, sudo bloqué |
| **Secret de salle au repos** | Lecture du secret par un autre user local | `~/.config/partagpu/room.json` en `chmod 600` |
| **Pare-feu automatique** | Exposition réseau inutile | Port ouvert uniquement quand le partage est actif |
| **Anti-spoofing mDNS** | Flood, usurpation d'identité | Rate limiting, max peers, détection de conflits |
| **Cap connexions peer-API** | OOM par flood TCP sur le port 7655 | `Semaphore(64)` acquis avant `accept()` |
| **Anti-CSRF API locale** | Page web hostile pivotant sur `127.0.0.1:7654` | Refus si `Host` ≠ `127.0.0.1:7654` ou `Origin` présent (bloque DNS rebinding) |
| **CSP webview** | XSS / injection HTML dans la webview Tauri | `default-src 'self'` + `frame-ancestors 'none'` + `object-src 'none'` |
| **PolicyKit** | Escalade de privilèges | Helper Rust compilé, mot de passe via stdin |
| **Validation des entrées** | Injection de commandes | Allowlist, validation stricte, pas de shell |
| **Passphrase masquée (UX)** | Fuite visuelle du code de salle | Étoiles par défaut, révèle uniquement tant que l'œil est maintenu |

---

## 1. Authentification des pairs par HMAC + timestamp

### Le problème

Sur un réseau local, n'importe qui peut annoncer un service mDNS et se faire passer pour un pair PartaGPU légitime. Sans vérification, un attaquant pourrait soumettre des tâches malveillantes à n'importe quelle machine.

### La solution

Chaque salle PartaGPU partage un **secret cryptographique** (encodé comme un code de 4 mots). De ce secret on dérive deux clés :

- une **`room_key`** pour le chiffrement AES-256-GCM des bodies, dérivée via HKDF-SHA256 (cf. section 2)
- une **`auth_key`** distincte pour les preuves d'authentification HMAC, dérivée via **PBKDF2-HMAC-SHA256** avec 600 000 itérations (slow KDF)

Pour la **vérification des pairs**, chaque appli sonde `/peer/v1/verify?nonce=<hex>` sur les pairs découverts en mDNS. Le pair répond avec `HMAC-SHA256(auth_key, "PartaGPU/verify-resp/v1\n" || nonce_bytes)` ; le sondeur recompute et compare en temps constant pour flipper le badge `verified`. **Aucune preuve statique n'est broadcastée** sur le réseau — un attaquant passif sur le LAN ne peut pas collecter de tags HMAC périodiques pour brute-forcer la passphrase offline.

Pour les **requêtes HTTP**, chaque appel pair-à-pair embarque un header :
```
X-PartaGPU-AUTH: <unix_ts>:<HMAC-SHA256(auth_key, "PartaGPU/auth-req/v1\n" || ts || "\n" || method || "\n" || path || "\n" || sha256(body)) hex>
```

Le serveur vérifie que `|now - ts| ≤ 30 s` puis recalcule le HMAC et compare en temps constant. Le HMAC **lie l'auth au corps de la requête**, donc un header capté ne peut pas être rejoué sur une requête différente même dans la fenêtre de 30 s. Un attaquant qui ne connaît pas la `auth_key` n'arrive jamais à la couche AES — l'auth gate est validée *avant* le déchiffrement.

![Vérification d'auth des pairs](docs/images/security-auth-flow.svg)

### Détails techniques

- **Primitive** : HMAC-SHA256 (RFC 2104).
- **Tolérance de clock skew** : ±1 fenêtre (`AUTH_WINDOW_MS = 30 000 ms`). Granularité **milliseconde** (pas seconde) pour que les requêtes de polling rapide (le SDK Python `live=True` poll à 250 ms) génèrent des timestamps distincts et ne soient pas rejetées par le cache anti-replay.
- **Anti-replay** : chaque header HMAC accepté est mémorisé pendant `2 × AUTH_WINDOW_MS / 1000 = 60 s`. Un header byte-identique reçu une seconde fois dans cette fenêtre est rejeté en 409. Cap dur de 4096 entrées pour éviter qu'un flood ne fasse exploser la RAM (au-delà, le cache est purgé — perte de garantie temporaire mais bornée).
- **Code d'accès** : 4 mots parmi 256 = 256^4 ≈ 4,3 milliards de combinaisons.
- **Conversion** : la passphrase est convertie en 4 octets, puis étendue à 20 octets via SHA-1 pour former un secret de longueur stable.
- **Dérivation** : `auth_key = PBKDF2-HMAC-SHA256(room_secret, salt = "PartaGPU/auth-key-pbkdf2-v2", iters = 600 000, len = 32 octets)`. Slow KDF intentionnel : la dérivation prend ~100 ms sur un CPU moderne, invisible au join de salle, mais multiplie d'un facteur ~10⁵ le coût d'un brute-force offline du passphrase à partir d'un tag HMAC observé. Distincte de la `room_key` AES dérivée par HKDF-SHA256 (la `room_key` n'est jamais broadcastée — pas le même profil de menace).
- **Verify endpoint** : `GET /peer/v1/verify?nonce=<hex>` est unauthenticated par design (c'est le bootstrap d'auth). Il accepte un nonce 16-32 octets en hex et renvoie `HMAC-SHA256(auth_key, "PartaGPU/verify-resp/v1\n" || nonce_bytes)` complet (256 bits, pas tronqué). Combiné avec le slow KDF, collecter des tags via probes répétés ne raccourcit pas le brute-force.
- **Persistance** : seul le secret est sauvegardé dans `~/.config/partagpu/room.json` ; la `auth_key` est dérivée à chaque chargement.

### Ce qui est bloqué

Quand une salle est active, le serveur peer-API :
- **Refuse** (HTTP 401) les requêtes sans header `X-PartaGPU-AUTH`
- **Refuse** (401) les requêtes avec header HMAC invalide (mauvaise clé, body altéré, timestamp hors fenêtre)
- **Refuse** (403) les requêtes quand le partage est désactivé localement
- **Logue** chaque rejet via `SecurityLog::peer_event(EventCategory::TaskRejected, …)`

### Fichiers concernés

- `src-tauri/src/auth.rs` — génération/vérification HMAC, passphrase, persistance
- `src-tauri/src/discovery.rs` — annonce et vérification du code dans les properties mDNS
- `src-tauri/src/api.rs` — vérification du pair dans `submit_task`

---

## 2. Chiffrement des messages pair-à-pair

### Le problème

L'auth HMAC authentifie le pair, mais ne chiffre rien. Sans chiffrement, un attaquant qui écoute le LAN (port mirror, ARP spoofing, ou simplement Wi-Fi partagé) verrait passer en clair :

- les arguments des commandes (`python3 -c "secret"` → secret visible)
- les fichiers du workspace pushés vers le pair (code propriétaire, datasets)
- les outputs stdout/stderr des tâches (résultats de calcul, parfois sensibles)

### La défense

Tous les bodies HTTP échangés sur le port pair-à-pair (`7655`, sauf `/peer/v1/health`) sont chiffrés en **AES-256-GCM** (chiffrement authentifié — confidentialité + intégrité dans une seule primitive).

#### Dérivation de la clé (v=1, fallback)

```
key = HKDF-SHA256(
    ikm    = base32_decode(room_secret),
    salt   = "PartaGPU/peer-api/v1",
    info   = "AES-256-GCM message key",
    length = 32 bytes,
)
```

Le `room_secret` est le même que celui qui sert à l'auth HMAC — déjà partagé entre membres de la salle via la passphrase de 4 mots. Aucun nouveau matériel à distribuer.

#### Dérivation de la clé (v=2, par défaut)

À chaque démarrage, chaque pair génère un keypair X25519 éphémère (gardé **uniquement en RAM**) et publie sa pubkey via mDNS (champ TXT `eph_pk`). Toutes les 10 minutes, un thread de fond fait tourner ce keypair et l'ancien reste valide ~60 s pour absorber les requêtes en vol.

Pour chaque requête, le client génère sa **propre** paire X25519 éphémère, calcule le secret partagé `ECDH(client_eph_priv, server_eph_pub)` et dérive la clé de session :

```
session_key = HKDF-SHA256(
    ikm    = ECDH_shared_secret,
    salt   = HKDF(room_secret),               // sert de salt en v=2
    info   = "AES-256-GCM session key v2 (room|ecdh)",
    length = 32 bytes,
)
```

La même clé sert pour la requête **et** la réponse — le serveur la dérive identiquement de son côté via `ECDH(server_eph_priv, client_eph_pub)`. La pubkey du client voyage avec l'envelope (`eph_pk`) ; la privkey ne quitte jamais sa machine.

C'est ce qui donne la **forward secrecy** : capturer le trafic chiffré + voler la passphrase de salle plus tard ne suffit plus, il faudrait aussi avoir la moitié privée d'un keypair éphémère qui n'a jamais été persisté.

#### Format

Deux versions d'enveloppes coexistent (le serveur accepte les deux ; le client envoie v=2 quand le pair publie sa pubkey, sinon v=1 pour les anciennes versions).

- **v=1 (legacy)** : `{"v": 1, "nonce": "<base64-12B>", "ct": "<base64>"}`. Clé AES = HKDF(room_secret).
- **v=2 (forward-secret, par défaut)** : `{"v": 2, "eph_pk": "<base64-32B>", "nonce": "...", "ct": "..."}`. Le client génère une paire X25519 éphémère par requête, fait Diffie-Hellman contre la pubkey éphémère du serveur (publiée en mDNS, **regénérée à chaque démarrage de l'app et tournée toutes les 10 minutes**, **jamais sur disque**), et la clé AES est HKDF(room_secret || ECDH_shared). La réponse réutilise la même clé de session. Lors d'une rotation, l'ancienne clé reste valide ~60 s pour les requêtes en vol.

Content-Type dans les deux cas : `application/x-partagpu-encrypted-v1`. Nonce de 12 octets random par message (largement sous le birthday bound de 2^48).

#### Mandatory

Le serveur peer-API rejette en `415 Unsupported Media Type` toute requête avec un body sans le bon Content-Type. Pas de fallback en clair — un attaquant qui essaie d'éviter la couche de chiffrement échoue ici.

### Propriétés

- **Confidentialité** : un attaquant qui écoute le trafic ne lit ni les commandes, ni les workspaces, ni les outputs.
- **Intégrité** : tout flip de bit dans un ciphertext fait échouer le déchiffrement (tag GCM rejeté). Le serveur retourne 415, le client reçoit l'erreur sans avoir accepté le message altéré.
- **Authenticité au niveau salle** : seuls les détenteurs du secret peuvent produire un body qui se déchiffre. le header `X-PartaGPU-AUTH` ajoute l'anti-replay sur ~30 s.
- **Forward secrecy (v=2)** : un attaquant qui capture du trafic chiffré et obtient le secret de salle plus tard ne peut pas déchiffrer ce qu'il a capturé, car la moitié privée de la clé éphémère n'a jamais quitté la RAM du serveur et a disparu au redémarrage de l'app.

### Limites connues

- **Forward secrecy bornée à 10 min** : un attaquant qui accède à la RAM d'un poste **pendant qu'il tourne** peut déchiffrer les sessions des 10 dernières minutes. Au-delà, les anciennes clés ont été rotées et écrasées.
- **Pas de protection contre un membre de la salle** : par construction, tout pair dans la salle a la clé. Le modèle de menace est "attaquant LAN qui n'est PAS dans la salle".
- **Anti-DOS faible** : le body (jusqu'à 32 MB) est lu et tenté de déchiffrer AVANT le check du HMAC. Un attaquant LAN pourrait spammer des bodies invalides pour forcer des allocations mémoire. Mitigation actuelle : le port n'est ouvert que quand le partage est actif (firewall fermé sinon).

### Fichiers concernés

- `src-tauri/src/crypto.rs` — module de chiffrement (HKDF, AES-GCM, envelope serde)
- `src-tauri/src/peer_api.rs` — handler chiffrement/déchiffrement des bodies
- `src-tauri/src/http_api.rs` — chiffrement côté client (run_remote_blocking)

Tests :
- Unitaires (8 tests) : `cargo test --lib crypto::` — round-trip v=1 et v=2, mauvaise clé, tampering, JSON round-trip, mauvaise clé éphémère, rotation grace window, forward-secrecy après rotation.
- Intégration (5 tests) : `cargo test --test peer_api_e2e` — refus du plaintext, refus sans header X-PartaGPU-AUTH, refus mauvais secret, round-trip v=2 complet sur un vrai serveur localhost, 404 sur cancel inconnu.

---

## 3. Sandbox d'exécution (bubblewrap)

### Le problème

Les tâches de calcul sont des commandes exécutées sur la machine. Même avec un pair vérifié, une erreur ou une compromission pourrait mener à l'exécution de commandes destructrices (`rm -rf /`, reverse shell, exfiltration de données).

### La solution

Chaque tâche s'exécute dans un **sandbox bubblewrap** avec des restrictions strictes.

![Sandbox d'exécution](docs/images/security-sandbox.svg)

### Restrictions appliquées

| Restriction | Détail |
|------------|--------|
| **Filesystem** | `/usr`, `/lib`, `/bin`, `/etc` montés en **lecture seule**. Aucun accès aux home directories. |
| **Workspace** | `/workspace` et `/tmp` sont des tmpfs éphémères — détruits à la fin de la tâche. |
| **Réseau** | `--unshare-net` — aucune connexion réseau possible (pas d'exfiltration, pas de reverse shell). |
| **Processus** | `--unshare-pid` — la tâche ne voit que ses propres processus, pas ceux de l'hôte. |
| **Utilisateur** | Exécution sous l'UID/GID du compte `partagpu`. |
| **Cgroup** | La tâche est placée dans `/sys/fs/cgroup/partagpu/` avec les limites CPU/RAM/PIDs définies par les sliders. |
| **GPU (CUDA MPS)** | Si NVIDIA MPS est installé, le daemon `nvidia-cuda-mps-control` tourne sous l'UID `partagpu` et la tâche reçoit `CUDA_MPS_ACTIVE_THREAD_PERCENTAGE=<gpu_limit>` — cap réel sur les SM threads. Sans MPS, le slider GPU est advisory uniquement (limitation NVIDIA grand public, pas un choix PartaGPU). |
| **Timeout** | Chaque tâche a un délai maximum (défaut : 1 heure). Si dépassé, le processus est tué. |
| **Sortie** | stdout limité à 1 Mo, stderr à 256 Ko — empêche un remplissage mémoire par sortie infinie. |
| **Artefacts** | Total cumulé limité à 256 MiB par tâche (`MAX_ARTIFACT_TOTAL_BYTES`). Les fichiers demandés via `outputs=[...]` sont validés par `canonicalize() + starts_with(workspace_root)` avant lecture — un path traversal (`../etc/shadow`) ou un symlink fabriqué pendant la tâche pour pointer hors du workspace est rejeté silencieusement, pas inclus dans la réponse. |
| **Pas de shell** | Les commandes sont passées en `argv` direct (pas de `sh -c`). L'injection de commandes est structurellement impossible. |

### Allowlist de commandes

Seules les commandes **explicitement autorisées** peuvent être exécutées. Par défaut :

`python3`, `python`, `bash`, `sh`, `cat`, `grep`, `awk`, `sed`, `make`, `cmake`, `gcc`, `g++`, `rustc`, `cargo`, `julia`, `Rscript`, `nvidia-smi`

L'allowlist est configurable via l'API (`addToAllowlist` / `removeFromAllowlist`).

Si une commande n'est pas dans la liste, la tâche est **refusée avant même de lancer le sandbox** — pas de tentative d'exécution.

### Trust boundary explicite : un pair vérifié = exécution arbitraire dans le sandbox

L'allowlist par défaut inclut volontairement `bash`, `sh`, `gcc`, `g++`, `make`, `cmake`, `cargo`, `rustc` parce que ce sont les outils standard d'une session ML/data science (tâches `python3 train.py` qui appellent `gcc` pour compiler une extension C, projets Cargo, scripts shell, etc.). **Un pair authentifié peut donc exécuter du code arbitraire dans le sandbox cible.** C'est attendu — l'allowlist filtre les *erreurs de frappe* et les binaires inattendus, pas un attaquant déterminé en possession de la passphrase.

Les défenses qui restent face à un pair compromis ou malveillant **dans la salle** :

- **Le sandbox bubblewrap** : filesystem read-only, network unshare, cgroup CPU/RAM/PIDs, UID `partagpu` (jamais root, ni l'UID de l'utilisateur normal). Une tâche malveillante voit `/usr` et `/etc` en lecture seule, ne peut pas toucher au home de l'utilisateur, ni au reste du système hors de `/workspace` et `/tmp` (eux-mêmes éphémères).
- **Le compte `partagpu` durci** : SSH bloqué, sudo bloqué, shell restreint qui ne fait que lancer PartaGPU.
- **Le passthrough `/dev/nvidia*`** est read-write et reste un vecteur de privilege escalation si le driver NVIDIA a un CVE non corrigé. Garder le driver à jour est partie intégrante du modèle.
- **Le cap PIDs** (1024 par cgroup) coupe les fork bombs ; les caps CPU/RAM bornent l'utilisation des ressources.

Ce que ça veut dire concrètement : si la passphrase de salle fuite (capture, indiscrétion), un attaquant qui rejoint la salle peut exécuter du code dans le sandbox de chaque pair vérifié. La défense est l'isolation, pas le filtrage de commandes. **La sécurité de la passphrase est donc l'invariant à maintenir** — voir l'affichage masqué de la passphrase (RevealOnHold) et le `chmod 600` sur `room.json`.

### Fichiers concernés

- `src-tauri/src/sandbox.rs` — construction de la commande bwrap, allowlist, exécution
- `src-tauri/src/task_runner.rs` — orchestration des tâches, appel au sandbox
- **Dépendance système** : `bubblewrap` (`sudo apt install bubblewrap`)

---

## 4. Durcissement du compte partagpu

### Le problème

Le compte `partagpu` est un vrai compte utilisateur avec un mot de passe (nécessaire pour se connecter via l'écran de login sur le PC d'un absent). Par défaut, cela signifie un accès shell complet, la possibilité de SSH, de sudo, etc.

### La solution

Le compte est verrouillé par 5 mécanismes complémentaires.

![Durcissement du compte](docs/images/security-account.svg)

### Détail des protections

**Shell restreint** (`/usr/local/lib/partagpu/partagpu-shell`)

Un script qui ne fait qu'une chose : lancer PartaGPU puis quitter la session. Si quelqu'un tente `su -c "commande" partagpu`, le shell détecte le flag `-c` et refuse. Ce shell est enregistré dans `/etc/shells` pour que les display managers (GDM, LightDM) l'acceptent.

**SSH bloqué** (`/etc/ssh/sshd_config.d/partagpu-deny.conf`)

```
DenyUsers partagpu
```

Même avec le bon mot de passe, impossible de se connecter en SSH. `sshd` est rechargé automatiquement après l'écriture du fichier.

**sudo bloqué** (`/etc/sudoers.d/partagpu-deny`)

```
partagpu ALL=(ALL) !ALL
```

Le compte ne peut jamais utiliser sudo, même s'il était ajouté à un groupe privilegié.

**Home verrouillé** — `chmod 700` sur `/var/lib/partagpu`. Les autres utilisateurs de la machine ne peuvent pas lire les fichiers du compte.

**Expiration du mot de passe** — `chage --maxdays 90`. Le mot de passe expire après 90 jours, forçant une rotation régulière.

**Mot de passe via stdin** — Le mot de passe n'est jamais passé en argument CLI (visible dans `/proc/*/cmdline`). Il transite par stdin vers `chpasswd`.

### Fichiers concernés

- `src-tauri/helper/src/main.rs` — `cmd_create_user()`, `install_restricted_shell()`, `install_ssh_deny()`, `install_sudoers_deny()`

---

## 5. Gestion automatique du pare-feu

### Le problème

Le port d'écoute de PartaGPU (TCP 7654) ne devrait être ouvert que quand le partage est réellement actif. Le laisser ouvert en permanence expose la machine inutilement.

### La solution

Le helper ouvre et ferme le port automatiquement en fonction de l'état du partage.

![Gestion du pare-feu](docs/images/security-firewall.svg)

### Règles appliquées

| Action utilisateur | Pare-feu |
|---|---|
| **Activer** le partage | `ufw allow 7654/tcp` + `ufw allow 5353/udp` |
| **Pause** | `ufw delete allow 7654/tcp` (fermeture immédiate) |
| **Reprendre** | `ufw allow 7654/tcp` (réouverture) |
| **Désactiver** | `ufw delete allow 7654/tcp` |
| **Supprimer le compte** | Fermeture du port + suppression du cgroup |

Le port mDNS (5353/UDP) n'est pas fermé lors de la pause ou désactivation car d'autres services système peuvent en dépendre.

**Compatibilité** : le helper détecte automatiquement `ufw` ou tombe sur `iptables` en fallback. Si aucun pare-feu n'est détecté, l'opération est silencieusement ignorée.

### Fichiers concernés

- `src-tauri/helper/src/main.rs` — `cmd_open_port()`, `cmd_close_port()`
- `src-tauri/src/sharing.rs` — appels automatiques dans `enable()`, `pause()`, `resume()`, `disable()`

---

## 6. Protection contre le mDNS spoofing / flood

### Le problème

mDNS est un protocole basé sur le multicast, sans authentification native. Un attaquant sur le réseau local peut :
- **Flooder** de fausses annonces pour remplir la liste de pairs et saturer la mémoire
- **Usurper** le hostname d'une machine existante pour se faire passer pour elle

### La solution

Trois protections complémentaires dans le module de découverte.

![Protection mDNS](docs/images/security-mdns-protection.svg)

### Détail des protections

**Limite maximale de pairs (50)**

Au-delà de 50 pairs découverts, les nouvelles annonces mDNS sont ignorées. Chaque rejet est loggé : `SECURITY: max peers (50) reached, ignoring new peer: <name>`.

**Rate limiting (2 secondes)**

Les mises à jour d'un même pair espacées de moins de 2 secondes sont silencieusement droppées. Cela empêche un attaquant de flooder avec des mises à jour rapides pour saturer le CPU ou pousser de faux états.

**Détection de conflit de hostname**

Si deux adresses IP différentes annoncent le même hostname, le second est marqué `hostname_conflict`. Dans l'interface :
- Badge `!!` rouge dans la colonne Auth
- Ligne avec fond rouge subtil
- Alerte : "Conflit de hostname détecté — possible usurpation d'identité"

Loggé : `SECURITY: hostname conflict detected — « <hostname> » announced by <IP> but already known from another IP`.

### Fichiers concernés

- `src-tauri/src/discovery.rs` — toute la logique de protection dans `start_browsing()`

---

## 7. Élévation de privilèges sécurisée (PolicyKit)

### Le problème

Certaines opérations nécessitent les droits root (créer un utilisateur, configurer les cgroups, gérer le pare-feu). L'application tourne sous un compte utilisateur normal.

### La solution

Un **binaire helper Rust** séparé (`partagpu-helper`) est exécuté via `pkexec` (PolicyKit). Cela affiche une fenêtre de mot de passe native du système.

### Pourquoi un binaire Rust et pas un script bash ?

- **Pas d'interpréteur** : un binaire compilé ne dépend pas de bash, PATH, IFS, ou d'autres variables d'environnement manipulables
- **Typage fort** : les entrées sont validées par le compilateur et le code, pas par des regex bash fragiles
- **Pas d'injection** : les commandes sont exécutées via `Command::new()` avec des arguments séparés, jamais concaténés dans une chaîne shell
- **Zéro dépendance** : le helper n'utilise que la bibliothèque standard Rust

### Quand pkexec est-il appelé ?

`pkexec` n'est demandé que pour **4 opérations** :

| Commande | Quand |
|----------|-------|
| `create-user` | Première activation du partage |
| `set-password` | Définition/modification du mot de passe |
| `setup-cgroup` | Première création du cgroup (ensuite écriture directe) |
| `open-port` / `close-port` | Activation/désactivation du partage |

Les ajustements de sliders, le monitoring, et la vérification de statut **n'appellent jamais pkexec**. Les fichiers cgroup sont rendus modifiables par l'utilisateur courant après la première création (via `chown` du `PKEXEC_UID`).

L'option `auth_admin_keep` dans la policy PolicyKit mémorise le mot de passe quelques minutes, évitant de le redemander pour chaque opération successive.

### Fichiers concernés

- `src-tauri/helper/` — crate Rust du helper (zéro dépendance)
- `src-tauri/resources/com.partagpu.policy` — règle PolicyKit
- `src-tauri/src/user_manager.rs` — appels au helper via `pkexec`

---

## 8. Validation des entrées

Toutes les entrées utilisateur et réseau sont validées avant traitement :

### Mot de passe

- Longueur : 4–128 caractères
- Caractères interdits : null bytes (`\0`), retours chariot (`\r`, `\n`)
- Transmis via stdin (jamais en argument CLI)
- Validé côté Rust **et** côté helper

### Limites cgroup

- `cpu_percent` : plafonné à 100, validé comme entier positif
- `ram_limit_mb` : plafonné à 1 048 576 (1 To), validé comme entier positif
- `PKEXEC_UID` : validé comme entier avant d'être passé à `chown`

### Commandes de tâches

- Vérifiées contre l'allowlist **avant** toute exécution
- Passées en `argv` (tableau d'arguments), jamais comme chaîne shell
- Aucun shell n'est impliqué (`sh -c` n'est jamais appelé)

### Passphrase de salle

- Doit contenir exactement 4 mots séparés par des tirets
- Chaque mot est vérifié contre la wordlist de 256 mots
- Un mot inconnu produit un message d'erreur explicite

### Affichage masqué de la passphrase (UX)

La passphrase de salle n'est **jamais affichée en clair par défaut** dans l'interface : le composant `RevealOnHold` la rend sous la forme `*****-*****-****-*****` et exige que l'utilisateur **maintienne** un bouton œil (souris, tactile ou clavier Espace/Entrée) pour la révéler. Au relâchement (ou à la perte de focus), elle se re-masque immédiatement. Pas de toggle persistant : la passphrase ne peut pas rester affichée par accident — par exemple, si quelqu'un quitte temporairement son poste pendant la dictée du code aux camarades.

---

## Signaler une vulnérabilité

Si vous trouvez une vulnérabilité dans PartaGPU, merci de la signaler de manière responsable en ouvrant une issue privée sur le dépôt GitHub ou en contactant directement les mainteneurs. Ne publiez pas de détails d'exploitation publiquement avant qu'un correctif ne soit disponible.
