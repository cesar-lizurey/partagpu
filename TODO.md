# TODO — Sécurité

Mesures de sécurité restantes à implémenter. Les mesures déjà en place sont documentées dans [SECURITY.md](SECURITY.md).

## HAUT

### Chiffrer les communications entre pairs
- **Risque** : les échanges de tâches et de résultats transitent en clair sur le réseau local. Un attaquant peut intercepter, modifier ou rejouer les messages.
- **Défense** :
  - [ ] Dériver une clé AES symétrique du secret de salle (déjà partagé via le passphrase)
  - [ ] Chiffrer chaque message échangé entre pairs avec cette clé
  - [ ] Rejeter tout message non chiffré ou avec une clé invalide
