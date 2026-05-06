# TODO — Sécurité

Mesures de sécurité restantes à implémenter. Les mesures déjà en place sont documentées dans [SECURITY.md](SECURITY.md).

## Fait

- ✅ **Chiffrement des communications entre pairs** (depuis 1.6.0). AES-256-GCM avec clé dérivée HKDF-SHA256 du secret de salle. Voir [ARCHITECTURE.md → Chiffrement des messages pair-à-pair](docs/ARCHITECTURE.md#chiffrement-des-messages-pair-à-pair).
- ✅ **Forward secrecy** (depuis 1.6.0). Échange Diffie-Hellman X25519 éphémère par requête (envelope v=2). La clé éphémère du serveur est gardée en RAM uniquement et regénérée à chaque démarrage de l'application : un attaquant qui capture du trafic et obtient le secret de salle plus tard ne peut plus déchiffrer une fois l'app redémarrée.
- ✅ **Per-task cgroup isolation** (depuis 1.6.0). Chaque tâche reçoit son propre `/sys/fs/cgroup/partagpu/task-<uuid>` pour éviter qu'une tâche OOM les voisines.
- ✅ **Limite de tâches concurrentes** (depuis 1.6.0). Cap configurable depuis l'UI ; au-delà, les tâches restent en file d'attente FIFO.

## Reste à faire

### Forward secrecy : re-keying périodique en cours d'exécution
- **Risque** : la clé éphémère actuelle ne tourne qu'au redémarrage de l'app. Si un attaquant accède à la RAM d'un poste **pendant** qu'il tourne, il peut déchiffrer toutes les communications jusqu'au prochain redémarrage.
- **Défense** :
  - [ ] Régénérer `EphemeralKey` toutes les 10 minutes (ou après N requêtes), garder l'ancienne 30 s pour les requêtes en vol.
  - [ ] Re-publier la nouvelle pubkey via mDNS dès la rotation.
- **Priorité** : faible. Le modèle de menace classroom n'inclut pas l'attaque mémoire en direct.

### Tests d'intégration end-to-end
- **Manque** : seuls les tests unitaires du module crypto existent. Le flux complet (deux instances qui se découvrent par mDNS, s'authentifient, dispatchent une tâche) est testé à la main.
- **À faire** :
  - [ ] Test qui démarre deux instances locales sur ports différents et vérifie qu'une tâche dispatchée arrive bien et termine.
  - [ ] Test qui simule un pair non vérifié et vérifie que le dispatch est refusé.
- **Priorité** : moyenne. Utile avant de publier sur PyPI/.deb publiquement.
