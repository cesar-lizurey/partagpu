🇬🇧 [English version](TODO.en.md)

# TODO

Travail restant. Les mesures **déjà en place** ne sont pas listées ici — elles vivent dans la documentation (`SECURITY.md`, `docs/ARCHITECTURE.md`, `docs/RELEASING.md`).

---

## 🔴 Sécurité — chantier restant

### Brute-force offline du passphrase via mDNS — phase 2

- **Phase 1 (faite, 1.10.0)** : `derive_auth_key` est passé de HKDF (~1 µs/candidat) à PBKDF2-HMAC-SHA256 600 000 itérations (~100 ms/candidat). Le brute-force passive d'`auth_proof` mDNS est passé de ~10 min laptop à ~7 jours CPU = ~1 500 € de cloud.
- **Phase 2 (à faire)** : retirer entièrement `auth_proof` du TXT mDNS et déplacer la vérification dans une route `/peer/v1/verify` qui exige un challenge HMAC bidirectionnel rate-limité par IP source. Supprime le leak passif au lieu de le rendre coûteux.
- **Coût** : ~3-4 h de boulot (nouvelle route + refactor discovery pour probe async + état UI verified=false par défaut + tests).
- **Priorité** : moyenne (l'exposition résiduelle après phase 1 n'est plus exploitable au threat model salle de cours).

---

## ✅ Sécurité — réglés en 1.10.0

Liste courte avec les commits, gardée comme historique. Détails dans SECURITY.md / SECURITY.en.md.

| # | Item | Commit |
|---|------|--------|
| 1 | `room.json` en `chmod 600` (sinon `0644` par umask → autres users locaux lisent le secret) | `8fa7c33` |
| 2 | Origin/Host gate sur `127.0.0.1:7654` contre CSRF + DNS rebinding | `e6cc705` |
| 3 | Slow KDF (PBKDF2 600 k iters) sur la dérivation `auth_key` | `26a4c35` |
| 4 | `Semaphore(64)` de connexions concurrentes sur peer API | `44278d1` |
| 5 | `pids.max=1024` + contrôleur `pids` activé sur cgroup | `33bfb6f` |
| 6 | `ReplayCache` 60 s contre rejeu d'une requête capturée | `f4d69ed` |
| 7 | CSP stricte dans `tauri.conf.json` | `73acfed` |
| 8 | Trust boundary explicitée (pair vérifié = exécution arbitraire dans le sandbox, c'est attendu) | `dd8b8df` |

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
