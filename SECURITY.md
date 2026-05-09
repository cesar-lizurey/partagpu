🇬🇧 [English version](SECURITY.en.md)

# Sécurité de PartaGPU

Ce document détaille les mesures de sécurité implémentées dans PartaGPU. L'application est conçue pour fonctionner dans un environnement de salle de cours où les postes sont sur le même réseau local, avec un niveau de confiance modéré entre les utilisateurs.

---

## Table des matières

- [Vue d'ensemble](#vue-densemble)
- [1. Authentification des pairs par HMAC + timestamp](#1-authentification-des-pairs-par-hmac--timestamp)
- [2. Chiffrement des messages pair-à-pair](#2-chiffrement-des-messages-pair-à-pair)
- [3. Bac à sable d'exécution (*bubblewrap*)](#3-bac-à-sable-dexécution-bubblewrap)
- [4. Durcissement du compte partagpu](#4-durcissement-du-compte-partagpu)
- [5. Gestion automatique du pare-feu](#5-gestion-automatique-du-pare-feu)
- [6. Protection contre l'usurpation et l'inondation mDNS](#6-protection-contre-lusurpation-et-linondation-mdns)
- [7. Élévation de privilèges sécurisée (PolicyKit)](#7-élévation-de-privilèges-sécurisée-policykit)
- [8. Validation des entrées](#8-validation-des-entrées)
- [Signaler une vulnérabilité](#signaler-une-vulnérabilité)

---

## Vue d'ensemble

PartaGPU repose sur plusieurs couches de sécurité complémentaires :

| Couche | Protège contre | Implémentation |
|--------|---------------|----------------|
| **Authentification HMAC** | Pairs non autorisés, imposteurs | `auth_key` dérivée via PBKDF2 sur 600 000 itérations (KDF lente), défi actif sur `/peer/v1/verify` côté découverte, en-tête `X-PartaGPU-AUTH` lié au corps des requêtes |
| **Anti-rejeu** | Rejeu (*replay*) d'une requête capturée dans la fenêtre de 30 s | `ReplayCache` en mémoire qui déduplique les `X-PartaGPU-AUTH` déjà vus |
| **Chiffrement AES-256-GCM** | Écoute réseau passive | Clé HKDF du secret de salle plus ECDH X25519 à confidentialité persistante (*forward secrecy*), obligatoire sur `/peer/v1/tasks*` |
| **Bac à sable *bubblewrap*** | Exécution de code malveillant | Système de fichiers en lecture seule, pas de réseau, PID isolé, cgroup CPU/RAM/`pids.max=1024` (anti bombe à *fork*), plafond GPU sur les SM via CUDA MPS si disponible |
| **Compte durci** | Abus du compte partagpu | Shell restreint, SSH bloqué, sudo bloqué |
| **Secret de salle au repos** | Lecture du secret par un autre user local | `~/.config/partagpu/room.json` en `chmod 600` |
| **Pare-feu automatique** | Exposition réseau inutile | Port ouvert uniquement quand le partage est actif |
| **Protection mDNS** | Inondation (*flood*) et usurpation d'identité (*spoofing*) | Limitation de débit (*rate limiting*), nombre maximal de pairs, détection des conflits de nom d'hôte |
| **Plafond de connexions peer-API** | Saturation mémoire (*OOM*) par afflux TCP sur le port 7655 | `Semaphore(64)` acquis avant `accept()` |
| **Anti-CSRF API locale** | Page web hostile pivotant sur `127.0.0.1:7654` | Refus si `Host` ≠ `127.0.0.1:7654` ou si un `Origin` est présent (bloque le rebinding DNS) |
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

- une **`room_key`** pour le chiffrement AES-256-GCM des corps de requêtes, dérivée via HKDF-SHA256 (voir section 2)
- une **`auth_key`** distincte pour les preuves d'authentification HMAC, dérivée via **PBKDF2-HMAC-SHA256** avec 600 000 itérations (KDF lente)

Pour la **vérification des pairs**, chaque application sonde `/peer/v1/verify?nonce=<hex>` sur les pairs découverts par mDNS. Le pair répond avec `HMAC-SHA256(auth_key, "PartaGPU/verify-resp/v1\n" || nonce_bytes)` ; l'appelant recalcule la valeur attendue et la compare en temps constant pour faire basculer le badge `verified`. **Aucune preuve statique n'est diffusée** sur le réseau — un attaquant passif sur le LAN ne peut pas collecter de tags HMAC périodiques pour mener une attaque par force brute hors ligne sur la passphrase.

Pour les **requêtes HTTP**, chaque appel pair-à-pair embarque un en-tête :
```
X-PartaGPU-AUTH: <unix_ts>:<HMAC-SHA256(auth_key, "PartaGPU/auth-req/v1\n" || ts || "\n" || method || "\n" || path || "\n" || sha256(body)) hex>
```

Le serveur vérifie que `|now - ts| ≤ 30 s`, puis recalcule le HMAC et le compare en temps constant. Le HMAC **lie l'authentification au corps de la requête**, donc un en-tête capturé ne peut pas être rejoué sur une requête différente, même dans la fenêtre de 30 s. Un attaquant qui ne connaît pas la `auth_key` n'atteint jamais la couche AES — la barrière d'authentification est validée *avant* le déchiffrement.

![Vérification d'auth des pairs](docs/images/security-auth-flow.svg)

### Détails techniques

- **Primitive** : HMAC-SHA256 (RFC 2104).
- **Tolérance de décalage d'horloge** (*clock skew*) : ±1 fenêtre (`AUTH_WINDOW_MS = 30 000 ms`). Granularité à la **milliseconde** (et non à la seconde) afin que les requêtes de sondage rapide (le SDK Python interroge à 250 ms quand `live=True`) génèrent des horodatages distincts et ne soient pas rejetées par le cache anti-rejeu.
- **Anti-rejeu** : chaque en-tête HMAC accepté est mémorisé pendant `2 × AUTH_WINDOW_MS / 1000 = 60 s`. Un en-tête strictement identique (octet par octet) reçu une seconde fois dans cette fenêtre est rejeté en 409. Une borne stricte de 4096 entrées évite qu'un afflux ne fasse exploser la RAM ; au-delà, le cache est purgé — la garantie est alors temporairement perdue mais reste bornée dans le temps.
- **Code d'accès** : 4 mots parmi 256 = 256^4 ≈ 4,3 milliards de combinaisons.
- **Conversion** : la passphrase est convertie en 4 octets, puis étendue à 20 octets via SHA-1 pour former un secret de longueur stable.
- **Dérivation** : `auth_key = PBKDF2-HMAC-SHA256(room_secret, salt = "PartaGPU/auth-key-pbkdf2-v2", iters = 600 000, len = 32 octets)`. La lenteur de la KDF est intentionnelle : la dérivation prend environ 100 ms sur un CPU moderne — invisible au moment de rejoindre la salle, mais elle multiplie par un facteur de l'ordre de 10⁵ le coût d'une attaque par force brute hors ligne sur la passphrase à partir d'un tag HMAC observé. Cette clé est distincte de la `room_key` AES dérivée par HKDF-SHA256 (la `room_key` n'est jamais diffusée — le profil de menace n'est pas le même).
- **Point d'entrée de vérification** : `GET /peer/v1/verify?nonce=<hex>` n'est volontairement pas authentifié, puisqu'il sert à amorcer l'authentification elle-même. Il accepte un *nonce* de 16 à 32 octets en hexadécimal et renvoie `HMAC-SHA256(auth_key, "PartaGPU/verify-resp/v1\n" || nonce_bytes)` complet (256 bits, non tronqué). Combiné avec la KDF lente, collecter des tags via des sondages répétés ne raccourcit pas une attaque par force brute.
- **Persistance** : seul le secret est sauvegardé dans `~/.config/partagpu/room.json` ; la `auth_key` est dérivée à chaque chargement.

### Ce qui est bloqué

Quand une salle est active, le serveur peer-API :
- **refuse** (HTTP 401) les requêtes sans en-tête `X-PartaGPU-AUTH` ;
- **refuse** (401) les requêtes dont l'en-tête HMAC est invalide (mauvaise clé, corps altéré, horodatage hors fenêtre) ;
- **refuse** (403) les requêtes quand le partage est désactivé localement ;
- **journalise** chaque rejet via `SecurityLog::peer_event(EventCategory::TaskRejected, …)`.

### Fichiers concernés

- `src-tauri/src/auth.rs` — génération et vérification HMAC, passphrase, persistance
- `src-tauri/src/discovery.rs` — annonce et vérification du code dans les propriétés mDNS
- `src-tauri/src/api.rs` — vérification du pair dans `submit_task`

---

## 2. Chiffrement des messages pair-à-pair

### Le problème

L'authentification HMAC authentifie le pair mais ne chiffre rien. Sans chiffrement, un attaquant qui écoute le LAN (*port mirroring*, usurpation ARP — *ARP spoofing* —, ou simplement Wi-Fi partagé) verrait passer en clair :

- les arguments des commandes (`python3 -c "secret"` → secret visible) ;
- les fichiers du *workspace* envoyés vers le pair (code propriétaire, jeux de données) ;
- les sorties stdout et stderr des tâches (résultats de calcul, parfois sensibles).

### La défense

Tous les corps de requête HTTP échangés sur le port pair-à-pair (`7655`, sauf `/peer/v1/health`) sont chiffrés en **AES-256-GCM** (chiffrement authentifié — confidentialité et intégrité dans une seule primitive).

#### Dérivation de la clé (`v=1`, mode de repli)

```
key = HKDF-SHA256(
    ikm    = base32_decode(room_secret),
    salt   = "PartaGPU/peer-api/v1",
    info   = "AES-256-GCM message key",
    length = 32 bytes,
)
```

Le `room_secret` est le même que celui qui sert à l'auth HMAC — déjà partagé entre membres de la salle via la passphrase de 4 mots. Aucun nouveau matériel à distribuer.

#### Dérivation de la clé (`v=2`, par défaut)

À chaque démarrage, chaque pair génère une paire de clés X25519 éphémère (conservée **uniquement en RAM**) et publie sa clé publique via mDNS (champ TXT `eph_pk`). Toutes les 10 minutes, un fil d'arrière-plan fait tourner cette paire de clés ; l'ancienne reste valide environ 60 s pour absorber les requêtes en vol.

Pour chaque requête, le client génère sa **propre** paire X25519 éphémère, calcule le secret partagé `ECDH(client_eph_priv, server_eph_pub)` et dérive la clé de session :

```
session_key = HKDF-SHA256(
    ikm    = ECDH_shared_secret,
    salt   = HKDF(room_secret),               // sert de salt en v=2
    info   = "AES-256-GCM session key v2 (room|ecdh)",
    length = 32 bytes,
)
```

La même clé sert pour la requête **et** la réponse — le serveur la dérive de manière identique de son côté via `ECDH(server_eph_priv, client_eph_pub)`. La clé publique du client voyage avec l'enveloppe (champ `eph_pk`) ; la clé privée ne quitte jamais sa machine.

C'est ce qui assure la **confidentialité persistante** (*forward secrecy*) : capturer le trafic chiffré puis voler la passphrase de salle plus tard ne suffit plus, il faudrait aussi disposer de la clé privée d'une paire de clés éphémère qui n'a jamais été persistée sur disque.

#### Format

Deux versions du **format d'enveloppe** coexistent (interne au protocole `peer-API`, sans rapport avec la version de l'application). Le serveur accepte les deux ; le client envoie une enveloppe `v=2` lorsque le pair publie sa clé publique éphémère, sinon il retombe sur `v=1` pour rester compatible avec les versions antérieures de PartaGPU.

- **`v=1` (compatibilité, *legacy*)** : `{"v": 1, "nonce": "<base64-12B>", "ct": "<base64>"}`. La clé AES vaut `HKDF(room_secret)`.
- **`v=2` (à confidentialité persistante, par défaut)** : `{"v": 2, "eph_pk": "<base64-32B>", "nonce": "...", "ct": "..."}`. Le client génère une paire X25519 éphémère par requête, effectue un Diffie-Hellman avec la clé publique éphémère du serveur (publiée par mDNS, **régénérée à chaque démarrage de l'application et tournée toutes les 10 minutes**, **jamais sur disque**), puis dérive la clé AES via `HKDF(room_secret || ECDH_shared)`. La réponse réutilise la même clé de session. Lors d'une rotation, l'ancienne clé reste valide environ 60 s pour les requêtes en vol.

Le `Content-Type` est dans les deux cas `application/x-partagpu-encrypted-v1`. Chaque message utilise un *nonce* de 12 octets aléatoires (largement sous la borne d'anniversaire de 2^48).

#### Chiffrement obligatoire

Le serveur peer-API rejette avec un code `415 Unsupported Media Type` toute requête dont le corps n'a pas le bon `Content-Type`. Aucun repli en clair (*fallback*) n'est possible — un attaquant qui essaie de contourner la couche de chiffrement échoue ici.

### Propriétés

- **Confidentialité** : un attaquant qui écoute le trafic ne lit ni les commandes, ni les *workspaces*, ni les sorties.
- **Intégrité** : la moindre altération d'un bit dans le texte chiffré (*ciphertext*) fait échouer le déchiffrement (tag GCM rejeté). Le serveur retourne un code 415 et le client reçoit l'erreur sans avoir accepté le message altéré.
- **Authenticité au niveau de la salle** : seuls les détenteurs du secret peuvent produire un corps qui se déchiffre. L'en-tête `X-PartaGPU-AUTH` ajoute l'anti-rejeu sur environ 30 s.
- **Confidentialité persistante (`v=2`)** : un attaquant qui capture du trafic chiffré et obtient le secret de salle plus tard ne peut pas déchiffrer ce qu'il a capturé, car la clé privée éphémère n'a jamais quitté la RAM du serveur et a disparu au redémarrage de l'application.

### Limites connues

- **Confidentialité persistante bornée à 10 minutes** : un attaquant qui accède à la RAM d'un poste **pendant qu'il fonctionne** peut déchiffrer les sessions des dix dernières minutes. Au-delà, les anciennes clés ont été tournées et écrasées.
- **Pas de protection contre un membre de la salle** : par construction, tout pair dans la salle possède la clé. Le modèle de menace vise un « attaquant LAN qui n'est PAS dans la salle ».
- **Protection anti-déni-de-service limitée** : le corps de la requête (jusqu'à 32 Mo) est lu et le déchiffrement est tenté **avant** la vérification du HMAC. Un attaquant LAN pourrait inonder le serveur de corps invalides pour forcer des allocations mémoire. La mitigation actuelle : le port n'est ouvert que lorsque le partage est actif (le pare-feu est fermé sinon).

### Fichiers concernés

- `src-tauri/src/crypto.rs` — module de chiffrement (HKDF, AES-GCM, sérialisation de l'enveloppe)
- `src-tauri/src/peer_api.rs` — gestionnaire de chiffrement et de déchiffrement des corps de requête
- `src-tauri/src/http_api.rs` — chiffrement côté client (`run_remote_blocking`)

Tests :

- Unitaires (8 tests) : `cargo test --lib crypto::` — aller-retour `v=1` et `v=2`, mauvaise clé, altération du message, aller-retour JSON, mauvaise clé éphémère, fenêtre de tolérance lors d'une rotation, confidentialité persistante après rotation.
- Intégration (5 tests) : `cargo test --test peer_api_e2e` — refus du texte clair, refus sans en-tête `X-PartaGPU-AUTH`, refus avec un mauvais secret, aller-retour `v=2` complet sur un vrai serveur local, code 404 sur l'annulation d'une tâche inconnue.

---

## 3. Bac à sable d'exécution (*bubblewrap*)

### Le problème

Les tâches de calcul sont des commandes exécutées sur la machine. Même avec un pair vérifié, une erreur ou une compromission pourrait mener à l'exécution de commandes destructrices (`rm -rf /`, *reverse shell*, exfiltration de données).

### La solution

Chaque tâche s'exécute dans un **bac à sable *bubblewrap*** avec des restrictions strictes.

![Bac à sable d'exécution](docs/images/security-sandbox.svg)

### Restrictions appliquées

| Restriction | Détail |
|------------|--------|
| **Système de fichiers** | `/usr`, `/lib`, `/bin`, `/etc` montés en **lecture seule**. Aucun accès aux dossiers personnels (*home*) des utilisateurs. |
| **Workspace** | `/workspace` et `/tmp` sont des systèmes de fichiers tmpfs éphémères, détruits à la fin de la tâche. |
| **Réseau** | `--unshare-net` — aucune connexion réseau n'est possible (pas d'exfiltration, pas de *reverse shell*). |
| **Processus** | `--unshare-pid` — la tâche ne voit que ses propres processus, pas ceux de l'hôte. |
| **Utilisateur** | Exécution sous l'UID et le GID du compte `partagpu`. |
| **Cgroup** | La tâche est placée dans `/sys/fs/cgroup/partagpu/` avec les limites CPU, RAM et PIDs définies par les curseurs. |
| **GPU (CUDA MPS)** | Si NVIDIA MPS est installé, le daemon `nvidia-cuda-mps-control` s'exécute sous l'UID `partagpu` et la tâche reçoit `CUDA_MPS_ACTIVE_THREAD_PERCENTAGE=<gpu_limit>` — un plafond réel sur les unités SM. Sans MPS, le curseur GPU n'est qu'indicatif (limitation NVIDIA grand public, pas un choix PartaGPU). |
| **Délai d'expiration** | Chaque tâche a un délai maximum (par défaut : 1 heure). Si ce délai est dépassé, le processus est tué. |
| **Sortie** | stdout est limité à 1 Mo, stderr à 256 Ko — ce qui empêche un remplissage mémoire par sortie infinie. |
| **Artefacts** | Le total cumulé est limité à 256 MiB par tâche (`MAX_ARTIFACT_TOTAL_BYTES`). Les fichiers demandés via `outputs=[...]` sont validés par `canonicalize() + starts_with(workspace_root)` avant lecture — une traversée de chemin (*path traversal* — `../etc/shadow`) ou un lien symbolique fabriqué pendant la tâche pour pointer hors du *workspace* est rejeté silencieusement, sans être inclus dans la réponse. |
| **Pas de shell** | Les commandes sont passées directement comme `argv` (pas de `sh -c`). L'injection de commandes est structurellement impossible. |

### Liste d'autorisation des commandes (*allowlist*)

Seules les commandes **explicitement autorisées** peuvent être exécutées. Par défaut :

`python3`, `python`, `bash`, `sh`, `cat`, `grep`, `awk`, `sed`, `make`, `cmake`, `gcc`, `g++`, `rustc`, `cargo`, `julia`, `Rscript`, `nvidia-smi`

Cette liste est configurable via l'API (`addToAllowlist` et `removeFromAllowlist`).

Si une commande n'est pas dans la liste, la tâche est **refusée avant même que le bac à sable ne soit lancé** — il n'y a pas de tentative d'exécution.

### Frontière de confiance explicite : un pair vérifié peut exécuter du code arbitraire dans le bac à sable

La liste d'autorisation (*allowlist*) par défaut inclut volontairement `bash`, `sh`, `gcc`, `g++`, `make`, `cmake`, `cargo`, `rustc`, parce que ce sont les outils standard d'une session de *machine learning* ou de science des données (les tâches `python3 train.py` qui appellent `gcc` pour compiler une extension C, les projets Cargo, les scripts shell, etc.). **Un pair authentifié peut donc exécuter du code arbitraire dans le bac à sable cible.** C'est attendu : la liste d'autorisation filtre les *erreurs de frappe* et les binaires inattendus, pas un attaquant déterminé en possession de la passphrase.

Les défenses qui restent face à un pair compromis ou malveillant **dans la salle** :

- **Le bac à sable *bubblewrap*** : système de fichiers en lecture seule, réseau isolé, cgroup CPU/RAM/PIDs, UID `partagpu` (jamais *root*, ni l'UID de l'utilisateur normal). Une tâche malveillante voit `/usr` et `/etc` en lecture seule, ne peut pas toucher au dossier personnel de l'utilisateur, ni au reste du système hors de `/workspace` et `/tmp` (eux-mêmes éphémères).
- **Le compte `partagpu` durci** : SSH bloqué, sudo bloqué, shell restreint qui ne fait que lancer PartaGPU.
- **L'accès direct (*passthrough*) à `/dev/nvidia*`** est en lecture-écriture et reste un vecteur d'escalade de privilèges si le driver NVIDIA présente une vulnérabilité (CVE) non corrigée. Maintenir le driver à jour fait partie intégrante du modèle.
- **Le plafond de PIDs** (1024 par cgroup) coupe les bombes à *fork* ; les plafonds CPU et RAM bornent l'utilisation des ressources.

Concrètement, cela signifie que si la passphrase de salle fuite (capture, indiscrétion), un attaquant qui rejoint la salle peut exécuter du code dans le bac à sable de chaque pair vérifié. La défense repose sur l'isolation, pas sur le filtrage de commandes. **La sécurité de la passphrase est donc l'invariant à maintenir** — voir l'affichage masqué de la passphrase (composant `RevealOnHold`) et le `chmod 600` sur `room.json`.

### Fichiers concernés

- `src-tauri/src/sandbox.rs` — construction de la commande `bwrap`, liste d'autorisation, exécution
- `src-tauri/src/task_runner.rs` — orchestration des tâches, appel au bac à sable
- **Dépendance système** : *bubblewrap* (`sudo apt install bubblewrap`)

---

## 4. Durcissement du compte partagpu

### Le problème

Le compte `partagpu` est un vrai compte utilisateur avec un mot de passe (nécessaire pour se connecter via l'écran de login sur le PC d'un absent). Par défaut, cela signifie un accès shell complet, la possibilité de SSH, de sudo, etc.

### La solution

Le compte est verrouillé par 5 mécanismes complémentaires.

![Durcissement du compte](docs/images/security-account.svg)

### Détail des protections

**Shell restreint** (`/usr/local/lib/partagpu/partagpu-shell`)

C'est un script qui ne fait qu'une chose : lancer PartaGPU puis quitter la session. Si quelqu'un tente `su -c "commande" partagpu`, le shell détecte l'option `-c` et refuse. Ce shell est enregistré dans `/etc/shells` pour que les gestionnaires d'affichage (GDM, LightDM) l'acceptent.

**SSH bloqué** (`/etc/ssh/sshd_config.d/partagpu-deny.conf`)

```
DenyUsers partagpu
```

Même avec le bon mot de passe, impossible de se connecter en SSH. `sshd` est rechargé automatiquement après l'écriture du fichier.

**sudo bloqué** (`/etc/sudoers.d/partagpu-deny`)

```
partagpu ALL=(ALL) !ALL
```

Le compte ne peut jamais utiliser sudo, même s'il était ajouté à un groupe privilégié.

**Dossier personnel verrouillé** — `chmod 700` sur `/var/lib/partagpu`. Les autres utilisateurs de la machine ne peuvent pas lire les fichiers du compte.

**Expiration du mot de passe** — `chage --maxdays 90`. Le mot de passe expire après 90 jours, ce qui force une rotation régulière.

**Mot de passe via stdin** — le mot de passe n'est jamais passé en argument CLI (où il serait visible dans `/proc/*/cmdline`). Il transite par stdin vers `chpasswd`.

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

**Compatibilité** : le helper détecte automatiquement `ufw`, ou se rabat sur `iptables` à défaut. Si aucun pare-feu n'est détecté, l'opération est silencieusement ignorée.

### Fichiers concernés

- `src-tauri/helper/src/main.rs` — `cmd_open_port()`, `cmd_close_port()`
- `src-tauri/src/sharing.rs` — appels automatiques dans `enable()`, `pause()`, `resume()`, `disable()`

---

## 6. Protection contre l'usurpation et l'inondation mDNS

### Le problème

mDNS est un protocole multicast sans authentification native. Un attaquant sur le réseau local peut :

- **inonder** le réseau de fausses annonces (*flood*) pour remplir la liste des pairs et saturer la mémoire ;
- **usurper** (*spoof*) le nom d'hôte d'une machine existante pour se faire passer pour elle.

### La solution

Trois protections complémentaires dans le module de découverte.

![Protection mDNS](docs/images/security-mdns-protection.svg)

### Détail des protections

**Limite maximale de pairs (50)**

Au-delà de 50 pairs découverts, les nouvelles annonces mDNS sont ignorées. Chaque rejet est journalisé : `SECURITY: max peers (50) reached, ignoring new peer: <name>`.

**Limitation de débit (*rate limiting*, 2 secondes)**

Les mises à jour d'un même pair espacées de moins de 2 secondes sont silencieusement abandonnées. Cela empêche un attaquant d'inonder le serveur de mises à jour rapides pour saturer le CPU ou injecter de faux états.

**Détection de conflit de nom d'hôte**

Si deux adresses IP différentes annoncent le même nom d'hôte, le second est marqué `hostname_conflict`. Dans l'interface :

- un badge `!!` rouge apparaît dans la colonne Auth ;
- la ligne reçoit un fond rouge subtil ;
- une alerte s'affiche : « Conflit de nom d'hôte détecté — possible usurpation d'identité ».

Le message journalisé : `SECURITY: hostname conflict detected — « <hostname> » announced by <IP> but already known from another IP`.

### Fichiers concernés

- `src-tauri/src/discovery.rs` — toute la logique de protection dans `start_browsing()`

---

## 7. Élévation de privilèges sécurisée (PolicyKit)

### Le problème

Certaines opérations nécessitent les droits root (créer un utilisateur, configurer les cgroups, gérer le pare-feu). L'application tourne sous un compte utilisateur normal.

### La solution

Un **binaire helper Rust** séparé (`partagpu-helper`) est exécuté via `pkexec` (PolicyKit). Cela affiche une fenêtre de mot de passe native du système.

### Pourquoi un binaire Rust et non un script bash ?

- **Pas d'interpréteur** : un binaire compilé ne dépend ni de bash, ni du PATH, ni de l'IFS, ni d'autres variables d'environnement manipulables.
- **Typage fort** : les entrées sont validées par le compilateur et le code, pas par des expressions régulières bash fragiles.
- **Pas d'injection** : les commandes sont exécutées via `Command::new()` avec des arguments séparés, jamais concaténés dans une chaîne shell.
- **Zéro dépendance** : le helper n'utilise que la bibliothèque standard Rust.

### Quand pkexec est-il appelé ?

`pkexec` n'est demandé que pour **4 opérations** :

| Commande | Quand |
|----------|-------|
| `create-user` | Première activation du partage |
| `set-password` | Définition/modification du mot de passe |
| `setup-cgroup` | Première création du cgroup (ensuite écriture directe) |
| `open-port` / `close-port` | Activation/désactivation du partage |

Les ajustements de curseurs, la supervision et la vérification du statut **n'appellent jamais pkexec**. Les fichiers cgroup sont rendus modifiables par l'utilisateur courant après la première création (via `chown` à partir de `PKEXEC_UID`).

L'option `auth_admin_keep` dans la règle PolicyKit mémorise le mot de passe pendant quelques minutes, ce qui évite de le redemander pour chaque opération successive.

### Fichiers concernés

- `src-tauri/helper/` — *crate* Rust du helper (zéro dépendance)
- `src-tauri/resources/com.partagpu.policy` — règle PolicyKit
- `src-tauri/src/user_manager.rs` — appels au helper via `pkexec`

---

## 8. Validation des entrées

Toutes les entrées utilisateur et réseau sont validées avant traitement :

### Mot de passe

- Longueur : 4 à 128 caractères.
- Caractères interdits : octets nuls (`\0`), retours chariot (`\r`, `\n`).
- Transmis via stdin (jamais en argument CLI).
- Validé côté Rust **et** côté helper.

### Limites cgroup

- `cpu_percent` : plafonné à 100, validé comme entier positif.
- `ram_limit_mb` : plafonné à 1 048 576 (1 To), validé comme entier positif.
- `PKEXEC_UID` : validé comme entier avant d'être passé à `chown`.

### Commandes de tâches

- Vérifiées contre la liste d'autorisation **avant** toute exécution.
- Passées comme `argv` (tableau d'arguments), jamais comme chaîne shell.
- Aucun shell n'est impliqué (`sh -c` n'est jamais appelé).

### Passphrase de salle

- Elle doit contenir exactement 4 mots séparés par des tirets.
- Chaque mot est vérifié contre le dictionnaire (*wordlist*) de 256 mots.
- Un mot inconnu produit un message d'erreur explicite.

### Affichage masqué de la passphrase (UX)

La passphrase de salle n'est **jamais affichée en clair par défaut** dans l'interface : le composant `RevealOnHold` la rend sous la forme `*****-*****-****-*****` et exige que l'utilisateur **maintienne** un bouton en forme d'œil (souris, tactile ou clavier Espace/Entrée) pour la révéler. Au relâchement (ou à la perte du focus), elle se masque à nouveau immédiatement. Aucune bascule persistante n'existe : la passphrase ne peut pas rester affichée par accident — par exemple si quelqu'un quitte temporairement son poste pendant la dictée du code aux camarades.

---

## Signaler une vulnérabilité

Si vous trouvez une vulnérabilité dans PartaGPU, merci de la signaler de manière responsable en ouvrant un *issue* privé sur le dépôt GitHub, ou en contactant directement les mainteneurs. Ne publiez pas de détails d'exploitation publiquement avant qu'un correctif ne soit disponible.
