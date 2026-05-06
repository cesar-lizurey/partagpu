# TODO — Sécurité

Mesures de sécurité restantes à implémenter. Les mesures déjà en place sont documentées dans [SECURITY.md](SECURITY.md).

## Fait

- ✅ **Chiffrement des communications entre pairs** (depuis 1.6.0). AES-256-GCM avec clé dérivée HKDF-SHA256 du secret de salle. Voir [ARCHITECTURE.md → Chiffrement des messages pair-à-pair](docs/ARCHITECTURE.md#chiffrement-des-messages-pair-à-pair).

## Reste à faire

### Forward secrecy
- **Risque** : si le secret de salle leak un jour, tout l'historique réseau enregistré devient déchiffrable a posteriori.
- **Défense** :
  - [ ] Échange de clé éphémère par session (Diffie-Hellman X25519)
  - [ ] Re-keying périodique
- **Priorité** : moyenne. Pas critique pour le modèle "salle de cours" où le secret tourne avec la passphrase et les sessions sont courtes.

### Per-task cgroup isolation
- **Risque** : aujourd'hui toutes les tâches reçues partagent le cgroup `/sys/fs/cgroup/partagpu`. Une tâche peut consommer toute la RAM allouée et faire OOM les autres.
- **Défense** :
  - [ ] Créer `/sys/fs/cgroup/partagpu/task-<uuid>` par tâche
  - [ ] Sous-allouer les limites du cgroup parent
- **Priorité** : moyenne. Limite anti-DOS interne au compte partagpu.

### Limite de tâches concurrentes
- **Risque** : un pair pourrait recevoir 100 dispatches d'un coup, saturer CPU + spawner 100 bwrap.
- **Défense** :
  - [ ] Queue avec max-concurrent côté `IncomingTasks::create_and_run`
  - [ ] Tasks au-delà de la limite : retournent 429 Too Many Requests ou Queued (en attente)
- **Priorité** : faible. La salle est de confiance ; à voir si le projet sort du cadre cours.
