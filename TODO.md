🇬🇧 [English version](TODO.en.md)

# TODO

Aucun chantier en cours. Les mesures **déjà en place** vivent dans la documentation (`SECURITY.md`, `docs/ARCHITECTURE.md`, `docs/RELEASING.md`) — pas dans ce fichier.

---

## ✅ Sécurité — réglé en 1.10.0

Audit interne (threat-modeling « attaquant chevronné LAN ou local ») → 9 items, tous fixés. Détails dans `SECURITY.md` / `SECURITY.en.md` / `docs/ARCHITECTURE.md`.

| # | Item | Commit |
|---|------|--------|
| 1 | `room.json` en `chmod 600` | `8fa7c33` |
| 2 | Origin/Host gate sur `127.0.0.1:7654` (anti CSRF + DNS rebinding) | `e6cc705` |
| 3 | Slow KDF (PBKDF2 600 k iters) sur `derive_auth_key` — phase 1 du fix mDNS | `26a4c35` |
| 4 | `Semaphore(64)` de connexions concurrentes sur peer API | `44278d1` |
| 5 | `pids.max=1024` + contrôleur `pids` activé sur cgroup | `33bfb6f` |
| 6 | `ReplayCache` 60 s contre rejeu d'une requête capturée | `f4d69ed` |
| 7 | CSP stricte dans `tauri.conf.json` | `73acfed` |
| 8 | Trust boundary explicitée (pair vérifié = exécution arbitraire dans le sandbox, c'est attendu) | `dd8b8df` |
| 2-bis | Drop `auth_proof` du mDNS + endpoint `/peer/v1/verify` actif (challenge-response HMAC) — phase 2 du fix mDNS | `3b30761` |

## ✅ Tests d'intégration — réglé en 1.10.0

Ajout de deux tests e2e qui spawn deux instances peer-API réelles sur ports différents, partageant le même secret de salle :
- `two_instances_verify_each_other` — chaque pair répond correctement au challenge `/peer/v1/verify` de l'autre
- `two_instances_dispatch_end_to_end` — A chiffre une tâche, signe le HMAC, l'envoie à B ; B accepte ; A déchiffre la réponse

Le total de tests e2e passe de 9 à 11.
