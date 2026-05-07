🇬🇧 [English version](TODO.en.md)

# TODO — Sécurité

Mesures de sécurité restantes à implémenter. Les mesures déjà en place sont documentées dans [SECURITY.md](SECURITY.md).

## Fait

- ✅ **Chiffrement des communications entre pairs** (depuis 1.6.0). AES-256-GCM avec clé dérivée HKDF-SHA256 du secret de salle. Voir [ARCHITECTURE.md → Chiffrement des messages pair-à-pair](docs/ARCHITECTURE.md#chiffrement-des-messages-pair-à-pair).
- ✅ **Forward secrecy** (depuis 1.7.0). Échange Diffie-Hellman X25519 éphémère par requête (envelope v=2). La clé éphémère du serveur est gardée en RAM uniquement, regénérée à chaque démarrage **et tournée toutes les 10 minutes** ; la clé précédente reste valide ~60 s pour les requêtes en vol.
- ✅ **Per-task cgroup isolation** (depuis 1.6.0). Chaque tâche reçoit son propre `/sys/fs/cgroup/partagpu/task-<uuid>` pour éviter qu'une tâche OOM les voisines.
- ✅ **Limite de tâches concurrentes** (depuis 1.6.0). Cap configurable depuis l'UI ; au-delà, les tâches restent en file d'attente FIFO.
- ✅ **Tests d'intégration peer-API end-to-end** (depuis 1.7.0). 5 tests dans `src-tauri/tests/peer_api_e2e.rs` qui démarrent un vrai serveur sur 127.0.0.1:0 et vérifient : refus du plaintext, refus sans TOTP, refus avec un mauvais secret de salle, round-trip v=2 complet, 404 sur cancel d'une tâche inconnue.

## Reste à faire

Aucun chantier critique restant. Tout ce qui suit est de l'amélioration optionnelle.

### Tests d'intégration plus poussés
- **Manque** : les tests actuels couvrent le protocole côté pair (réception). Pas de test qui exerce le **dispatch** (deux instances qui se parlent réellement, l'une envoyant une tâche à l'autre).
- **Pourquoi c'est dur** : il faudrait aussi simuler le service mDNS (ou bypasser Discovery) pour qu'une instance trouve l'autre.
- **Priorité** : faible.

### Re-keying à granularité plus fine
- **État actuel** : la clé éphémère tourne toutes les 10 minutes. Suffisant pour le modèle classroom.
- **Amélioration possible** : tourner après N requêtes traitées (cap absolu sur la quantité de trafic chiffrée avec une même clé). Aucun bénéfice pratique au volume actuel.
- **Priorité** : nulle pour le projet actuel.

### Étendre la migration `thiserror` au reste du codebase
- **État actuel** : depuis 1.7.x, `crypto.rs` utilise un enum typé `CryptoError` avec variantes (`BadEncoding`, `BadLength`, `AeadDecrypt`, `MissingEphPk`, `Json`, `NoMatchingKey`…). Les callers (peer_api, http_api) convertissent encore via `.map_err(|e| e.to_string())` à la frontière car le reste du codebase est toujours sur `Result<T, String>`.
- **Pourquoi étendre** : permettrait aux handlers HTTP de pattern-matcher sur les variantes pour mapper vers des codes HTTP plus précis (415 vs 401 vs 500), au lieu d'un grep heuristique sur le message d'erreur. Permettrait aussi à terme de retirer les `format!()` qui inflate les erreurs en strings perdantes.
- **Coût** : large. ~100 sites dans tous les fichiers Rust (sandbox, task_runner, discovery, auth, http_api, peer_api). Mécanique mais fastidieux. Risque modéré de régression subtile (les unit tests couvrent peu de chemins d'erreur).
- **Bénéfice** : faible en pratique tant que personne ne pattern-match sur les erreurs côté UI. Surtout du nettoyage de design.
- **Couche Tauri** : restera sur `Result<T, String>` (les commands sérialisent les erreurs vers le JS) — la migration interne ne touche que le code Rust pur.
- **Priorité** : faible. À reprendre si on ajoute un consommateur d'erreurs typées (par ex. un endpoint `/peer/v1/error-summary` qui renvoie un code stable, ou des tests qui veulent assert sur une variante spécifique).

### Retirer TOTP au profit d'un HMAC + timestamp
- **État actuel** : l'auth des requêtes pair-à-pair s'appuie sur un code TOTP à 6 chiffres dans le header `X-PartaGPU-TOTP`, plus l'annonce du même code dans les TXT records mDNS pour la vérification passive entre pairs.
- **Pourquoi le remplacer** : depuis l'ajout d'AES-256-GCM (1.6.0) puis de la forward secrecy X25519 (1.7.0), TOTP ne fait plus que de l'anti-replay sur ~30 s. C'est exactement ce qu'un schéma plus standard apporterait :
  - HTTP : header `X-PartaGPU-AUTH: HMAC-SHA256(room_key, timestamp || body_hash)` + check côté serveur `|now - timestamp| < 30 s` → mêmes garanties anti-replay, plus lisible, pas de dépendance `totp-rs` / `base32`.
  - mDNS : broadcaster `HMAC-SHA256(room_key, current_time_window)` tronqué (qui *est* TOTP au sens mathématique mais sans le formalisme RFC 6238) ou un challenge HTTP léger pour la vérif.
- **Coût** : break du protocole pair-à-pair, donc force un upgrade simultané. Probablement ~1 jour de boulot incluant tests + migration de la doc.
- **Bénéfice** : réduction de la surface dépendances (`totp-rs`, `base32`, `data-encoding`), code plus lisible (HMAC explicite plutôt que TOTP qui est un HMAC déguisé), et moins de couches superposées qui font la même chose.
- **Priorité** : faible. Le système marche, retirer TOTP ne gagne rien sur le plan sécurité — c'est purement du nettoyage de design.
