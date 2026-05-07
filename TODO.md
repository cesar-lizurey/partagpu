🇬🇧 [English version](TODO.en.md)

# TODO

Travail restant. Les mesures **déjà en place** ne sont pas listées ici — elles vivent dans la documentation (`SECURITY.md`, `docs/ARCHITECTURE.md`, `docs/RELEASING.md`).

---

## 🔴 Sécurité — chantiers prioritaires

Issus d'un audit interne (threat-modeling « attaquant chevronné sur le LAN ou en local »).

### `room.json` lisible par tous les utilisateurs locaux

- **Problème** : `~/.config/partagpu/room.json` contient `secret_base32` en clair. `fs::write` n'applique pas de `chmod` explicite — le fichier hérite de l'umask par défaut (0644) → les autres utilisateurs locaux peuvent le lire.
- **Impact** : sur une machine multi-utilisateurs, un autre user lit le secret de salle complet et peut envoyer des tâches arbitraires aux pairs.
- **Fix** : `set_permissions(&path, 0o600)` après le `fs::write` dans `auth.rs::save_room`.
- **Priorité** : haute (1 ligne, ferme un trou évident).

### CSRF / DNS rebinding sur l'API locale `127.0.0.1:7654`

- **Problème** : `http_api.rs` répond `Access-Control-Allow-Origin: *` et ne vérifie ni `Origin` ni `Referer` ni `Host`. Toute page web ouverte dans le navigateur de la victime peut faire `fetch("http://127.0.0.1:7654/api/dispatch", ...)` et dispatcher des tâches sur les pairs vérifiés. DNS rebinding (`evil.com` → 127.0.0.1) bypasse même PNA sur Firefox.
- **Impact** : exécution de code arbitraire sur tous les pairs vérifiés depuis n'importe quel onglet ouvert sur la machine de la victime.
- **Fix** : refuser toute requête sans `Host: 127.0.0.1:7654` (ou dont l'`Origin` est non-vide et non-Tauri). Les invocations Tauri internes n'envoient pas d'`Origin` ; le client Python utilise `requests` avec `Host: 127.0.0.1:7654`.
- **Priorité** : haute.

### Brute-force offline du passphrase via mDNS — atténuation faite, redesign restant

- **Problème** : `crypto.rs::current_auth_proof` produit un HMAC tronqué à 32 bits (8 hex chars), broadcasté en clair en TXT mDNS. Un attaquant passif sur le LAN collecte 2-3 windows et brute-force offline les 256^4 ≈ 4.3 G passphrases possibles.
- **Phase 1 (faite, depuis 1.10.0)** : la dérivation `auth_key` est passée de HKDF (~1 µs/candidat) à PBKDF2-HMAC-SHA256 600 000 itérations (~100 ms/candidat). Le brute-force passe de ~10 minutes laptop à ~7 jours CPU = ~1 500 € de cloud — infeasible au modèle « camarade curieux ».
- **Phase 2 (à faire)** : retirer entièrement `auth_proof` du TXT mDNS et déplacer la vérification dans une route `/peer/v1/verify` qui exige un challenge HMAC bidirectionnel rate-limité par IP source. Élimine le leak passif au lieu de le rendre coûteux.
- **Priorité** : moyenne (le risque pratique a chuté d'un facteur 10⁵, l'exposition restante n'est plus exploitable au threat model salle de cours).

<!-- Items réglés depuis l'audit — gardés ici comme historique court avant
nettoyage final. Les détails techniques vivent dans les commits / SECURITY.md. -->

### ✅ Cap de connexions concurrentes sur peer API

Réglé en 1.10.0 : `tokio::sync::Semaphore` de 64 permits acquis avant `accept()`. Au-delà, le surplus est absorbé par le TCP backlog du kernel.

### ✅ `pids.max` sur cgroup

Réglé en 1.10.0 : contrôleur `pids` activé dans `subtree_control` + `pids.max=1024` sur le cgroup parent (héritage automatique aux sub-cgroups par tâche).

### ✅ Anti-replay au-delà de la fenêtre de timestamp

Réglé en 1.10.0 : `ReplayCache` en mémoire dans `peer_api.rs`. Après auth réussie, le header `X-PartaGPU-AUTH` est inséré dans la cache (rétention 60 s, cap 4096 entrées, clear si débordement). Replay byte-identique → 401.

### ✅ CSP stricte dans Tauri

Réglé en 1.10.0 : `tauri.conf.json` passe de `"csp": null` à une politique stricte (`default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:; connect-src 'self' ipc: …; object-src 'none'; base-uri 'self'; frame-ancestors 'none'`).

### ✅ Trust boundary documentée

Réglé en 1.10.0 : section dédiée dans `SECURITY.md` / `SECURITY.en.md` qui explicite « un pair vérifié peut exécuter du code arbitraire dans le sandbox cible, c'est attendu ; les défenses sont l'isolation (sandbox + compte durci + cgroup), pas le filtrage de commandes ».

---

## 🟢 Améliorations non-sécurité (priorité faible)

### Tests d'intégration plus poussés

- **Manque** : les tests actuels couvrent le protocole côté pair (réception). Pas de test qui exerce le **dispatch end-to-end** (deux instances qui se parlent réellement, l'une envoyant une tâche à l'autre).
- **Pourquoi c'est dur** : il faudrait aussi simuler le service mDNS (ou bypasser `Discovery`) pour qu'une instance trouve l'autre.
- **Priorité** : faible.

### Étendre la migration `thiserror` au reste du codebase

- **État actuel** : `crypto.rs` utilise un enum typé `CryptoError` (depuis 1.7.x). Le reste du codebase est toujours sur `Result<T, String>`.
- **Pourquoi étendre** : permettrait aux handlers HTTP de pattern-matcher sur les variantes pour mapper vers des codes HTTP plus précis (415 vs 401 vs 500), au lieu d'un grep heuristique sur le message d'erreur.
- **Coût** : ~100 sites à toucher. Mécanique mais fastidieux. Risque modéré de régression subtile.
- **Bénéfice** : faible en pratique tant que personne ne pattern-match sur les erreurs côté UI. Surtout du nettoyage de design.
- **Couche Tauri** : restera sur `Result<T, String>` (les commands sérialisent les erreurs vers le JS).
- **Priorité** : faible. À reprendre si un consommateur d'erreurs typées apparaît.

### Re-keying à granularité plus fine

- **État actuel** : la clé éphémère X25519 tourne toutes les 10 minutes (cf. SECURITY.md).
- **Amélioration possible** : tourner aussi après N requêtes traitées (cap absolu sur la quantité de trafic chiffrée avec une même clé).
- **Bénéfice** : aucun bénéfice pratique au volume actuel.
- **Priorité** : nulle pour le projet actuel.
