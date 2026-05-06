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
