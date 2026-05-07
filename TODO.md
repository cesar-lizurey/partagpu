🇬🇧 [English version](TODO.en.md)

# TODO

Travail restant. Les mesures **déjà en place** ne sont pas listées ici — elles vivent dans la documentation (`SECURITY.md`, `docs/ARCHITECTURE.md`, `docs/RELEASING.md`).

Aucun chantier critique. Tout ce qui suit est de l'amélioration optionnelle, classée par valeur décroissante.

## Tests d'intégration plus poussés

- **Manque** : les tests actuels couvrent le protocole côté pair (réception). Pas de test qui exerce le **dispatch end-to-end** (deux instances qui se parlent réellement, l'une envoyant une tâche à l'autre).
- **Pourquoi c'est dur** : il faudrait aussi simuler le service mDNS (ou bypasser `Discovery`) pour qu'une instance trouve l'autre.
- **Priorité** : faible.

## Étendre la migration `thiserror` au reste du codebase

- **État actuel** : `crypto.rs` utilise un enum typé `CryptoError` (depuis 1.7.x). Le reste du codebase est toujours sur `Result<T, String>`.
- **Pourquoi étendre** : permettrait aux handlers HTTP de pattern-matcher sur les variantes pour mapper vers des codes HTTP plus précis (415 vs 401 vs 500), au lieu d'un grep heuristique sur le message d'erreur.
- **Coût** : ~100 sites à toucher. Mécanique mais fastidieux. Risque modéré de régression subtile.
- **Bénéfice** : faible en pratique tant que personne ne pattern-match sur les erreurs côté UI. Surtout du nettoyage de design.
- **Couche Tauri** : restera sur `Result<T, String>` (les commands sérialisent les erreurs vers le JS).
- **Priorité** : faible. À reprendre si un consommateur d'erreurs typées apparaît.

## Re-keying à granularité plus fine

- **État actuel** : la clé éphémère X25519 tourne toutes les 10 minutes (cf. SECURITY.md).
- **Amélioration possible** : tourner aussi après N requêtes traitées (cap absolu sur la quantité de trafic chiffrée avec une même clé).
- **Bénéfice** : aucun bénéfice pratique au volume actuel.
- **Priorité** : nulle pour le projet actuel.
