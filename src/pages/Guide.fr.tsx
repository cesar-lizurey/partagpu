import { version as APP_VERSION } from "../../package.json";

export function GuideFr() {
  return (
    <div className="page guide">
      <h2>Guide — Comment ça marche ?</h2>
      <p className="page__subtitle">PartaGPU v{APP_VERSION}</p>

      <section className="guide__section">
        <h3>1. Créer ou rejoindre une salle</h3>
        <p>
          La <strong>salle</strong> est une enceinte cryptographique partagée :
          tous les postes qui en font partie peuvent s'envoyer (<em>dispatch</em>) du calcul
          entre eux, et les postes extérieurs sont automatiquement refusés.
        </p>
        <ul>
          <li>
            <strong>Créer une salle</strong> (un seul élève le fait) : cliquer sur le bouton{" "}
            <em>« Créer une salle »</em>, puis choisir un nom (par exemple{" "}
            <em>« Salle B204 »</em>). L'application génère alors un code d'accès de 4 mots
            (<code>pomme-tigre-bleu-ocean</code>). Ce code est affiché masqué
            (<code>*****-*****-****-*****</code>) ; il suffit de maintenir l'icône d'œil pour
            le révéler le temps de le dicter à voix haute.
          </li>
          <li>
            <strong>Rejoindre une salle</strong> (tous les autres) : cliquer sur le bouton{" "}
            <em>« Rejoindre une salle »</em>, puis entrer le même nom et le code
            de 4 mots dicté.
          </li>
        </ul>
        <p>
          Les pairs vérifiés affichent un badge vert <strong>OK</strong>.
          Ceux d'une autre salle (ou qui n'en ont rejoint aucune) affichent un{" "}
          <strong>?</strong> rouge — vous ne pouvez pas leur envoyer de
          tâches.
        </p>
      </section>

      <section className="guide__section">
        <h3>2. Activer le partage sur cette machine</h3>
        <p>
          Dans l'onglet <strong>« Mon partage »</strong>, cliquer sur le bouton{" "}
          <em>« Activer »</em>. Une fenêtre PolicyKit demande le mot de passe
          de la session courante (et non un mot de passe administrateur
          distinct). L'application crée alors le compte système{" "}
          <code>partagpu</code>, configure son cgroup v2 et ouvre le pare-feu
          sur les ports nécessaires.
        </p>
        <p>
          Définissez ensuite le <strong>mot de passe du compte partagpu</strong>{" "}
          dans le formulaire qui apparaît. C'est ce mot de passe qui permet de{" "}
          se connecter à la machine depuis l'écran de connexion (utile pour les
          ordinateurs d'élèves absents — voir plus bas).
        </p>
      </section>

      <section className="guide__section">
        <h3>3. Régler les limites de partage</h3>
        <p>
          Sur chaque jauge de ressource (CPU, RAM, GPU), un <strong>curseur</strong>{" "}
          indique la limite que vous partagez. Faites-le glisser à la souris pour
          ajuster sa valeur — la modification est appliquée instantanément par les
          <em> cgroups v2</em> du noyau, sans demander de mot de passe.
        </p>
        <p>
          La limite <strong>CPU</strong> est exprimée en pourcentage de la{" "}
          <strong>machine entière</strong>, et non d'un seul cœur : sur un
          poste à 16 cœurs, 50 % autorisent jusqu'à 8 cœurs cumulés pour les
          tâches partagées. La limite <strong>RAM</strong> est exprimée en Mo
          (la valeur 0 signifie illimitée).
        </p>
        <p>
          Les jauges affichent en plus la{" "}
          <strong>répartition par utilisateur</strong> : chaque barre est
          segmentée. Un segment vert (<em>« Vous (cette machine) »</em>)
          représente ce que vous consommez localement, puis chaque utilisateur
          distant qui consomme votre machine via une tâche PartaGPU apparaît
          dans un segment de couleur distincte. Le détail s'affiche au survol
          d'un segment.
        </p>
        <p>
          La jauge GPU peut afficher un avertissement{" "}
          <em>« indicative — CUDA MPS inactif »</em> à côté de la limite : cela
          signifie que le daemon NVIDIA MPS n'est pas en cours d'exécution,
          le plus souvent parce que <code>nvidia-cuda-mps-control</code> n'est
          pas installé. Tant que MPS n'est pas actif, le curseur GPU est{" "}
          <strong>purement informatif</strong> : la limite est annoncée aux
          pairs, mais le pilote CUDA ne la respecte pas, et une tâche peut
          saturer le GPU à 100 %. Pour activer une véritable application de
          la limite sur Ubuntu ou Debian :
        </p>
        <pre className="guide__code">
          <code>sudo apt install nvidia-cuda-toolkit</code>
        </pre>
        <p>
          Puis aller dans <em>Mon partage → Désactiver → Activer</em> pour que
          le helper relance le daemon MPS. L'avertissement disparaît alors et
          le curseur GPU devient un vrai contrat respecté côté CUDA. Avantage
          annexe sur les GPU récents (architectures <em>Ampere</em> et{" "}
          <em>Hopper</em>) : plusieurs tâches CUDA s'exécutent{" "}
          <strong>simultanément</strong> sur des SM différents, au lieu de se
          bloquer en partage de temps (<em>time-slicing</em>).
        </p>
        <p>
          Le champ <strong>« Tâches simultanées maximum »</strong> (réglé à 4
          par défaut) borne le nombre de tâches qui peuvent s'exécuter en même
          temps. Au-delà, les nouvelles tâches restent en file d'attente — c'est
          une protection contre un pair qui chercherait à inonder votre machine.
        </p>
      </section>

      <section className="guide__section">
        <h3>4. Soumettre des tâches</h3>
        <p>
          Tout se passe dans l'onglet <strong>« Mon utilisation »</strong>.
          Il existe trois façons d'envoyer (<em>dispatch</em>) du calcul :
        </p>
        <ul>
          <li>
            <strong>Une commande sur un pair</strong> : un formulaire accepte
            une commande shell ou un script Python téléversé. C'est l'option
            idéale pour un test ponctuel ou une tâche non distribuée.
          </li>
          <li>
            <strong>Entraînement DDP multi-machines</strong> : un panneau
            dédié permet de cocher plusieurs pairs et leurs GPU, de téléverser
            un script PyTorch, et calcule automatiquement{" "}
            <code>MASTER_ADDR</code>, <code>RANK</code>,{" "}
            <code>WORLD_SIZE</code>, <code>CUDA_VISIBLE_DEVICES</code>, etc.
            Un tableau affiche la progression par rang en direct, et un bouton{" "}
            <em>« Tout annuler »</em> propage l'arrêt à tous les rangs si l'un
            d'eux plante.
          </li>
          <li>
            <strong>Depuis Python</strong> : exécuter{" "}
            <code>pip install partagpu</code>, puis appeler{" "}
            <code>partagpu.run_remote(...)</code> ou{" "}
            <code>partagpu.distribute("train.py")</code> depuis un notebook.
            Trois options sont particulièrement utiles : <code>live=True</code>
            {" "}(les logs sont diffusés ligne par ligne pendant l'entraînement),{" "}
            <code>outputs=["model.pt"]</code> (rapatrie le <em>checkpoint</em>{" "}
            en RAM dans <code>results[0].artifacts</code>), et{" "}
            <code>local=False</code> (n'utilise que les pairs distants quand
            vous ne partagez pas votre propre machine). Voir la documentation
            du paquet Python pour le détail.
          </li>
        </ul>
      </section>

      <section className="guide__section">
        <h3>5. Suivi en temps réel</h3>
        <ul>
          <li>
            <strong>Tableau des tâches</strong> : il affiche le statut, la
            progression et l'utilisation CPU, RAM et GPU par tâche, mis à jour
            chaque seconde via les événements Tauri (sans sondage visible).
            Le tri place la <strong>plus récente en haut</strong> pour ne
            pas avoir à faire défiler la liste après un envoi.
          </li>
          <li>
            <strong>Durée des tâches</strong> sous la barre de progression :{" "}
            <code>↻ 2m 13s</code> pour une tâche active (rafraîchie à 1 Hz),
            puis <code>✓ 6m 42s</code> pour une tâche terminée (figée sur la
            durée totale). Les tâches encore en file d'attente n'affichent pas
            d'horloge.
          </li>
          <li>
            <strong>Bouton Stop</strong> sur chaque tâche en cours : un
            SIGTERM propre est envoyé côté pair, suivi d'un SIGKILL après 2 s
            si nécessaire. Pour les tâches DDP, l'annulation d'un rang se
            propage à tous les autres afin d'éviter le blocage de NCCL.
          </li>
          <li>
            <strong>Bouton 🗑 supprimer</strong> sur les tâches dans un état
            terminal (Terminée, Échouée ou Annulée) pour nettoyer
            l'historique. Le bouton refuse l'opération si la tâche est encore
            active — il faut d'abord l'annuler.
          </li>
          <li>
            <strong>Notifications système</strong> : à la fin d'un envoi de
            tâche (Terminée, Échouée ou Annulée), une notification système
            native s'affiche, même si l'application n'a pas le focus. C'est
            pratique pour s'éloigner pendant un long entraînement.
          </li>
          <li>
            <strong>Historique sur 5 minutes</strong> : sous les jauges de la
            page <em>« Mon partage »</em>, des mini-courbes
            (<em>sparklines</em>) montrent l'évolution du CPU, de la RAM et
            du GPU sur les cinq dernières minutes.
          </li>
        </ul>
      </section>

      <section className="guide__section">
        <h3>6. Vue parc</h3>
        <p>
          L'onglet <strong>« Vue parc »</strong> propose un tableau de bord
          agrégé de toutes les machines de la salle. Les statistiques
          globales sont affichées en haut (nombre de pairs visibles, GPU
          disponibles, votre consommation totale), puis une carte par pair
          présente sa capacité offerte et les tâches que vous y exécutez.
          Cette vue est utile pour superviser la salle d'un coup d'œil, par
          exemple pour un enseignant.
        </p>
      </section>

      <section className="guide__section">
        <h3>7. Pause ou Désactiver</h3>
        <p>Deux niveaux d'arrêt sont proposés, avec des sémantiques différentes :</p>
        <div className="guide__comparison">
          <div className="guide__card">
            <h4>Pause (temporaire)</h4>
            <ul>
              <li>Le pare-feu est fermé, les tâches entrantes sont refusées.</li>
              <li>Le compte <code>partagpu</code> est conservé.</li>
              <li>Le cgroup est conservé.</li>
              <li>Le venv géré est conservé.</li>
              <li>
                <em>Reprendre</em> est instantané et ne demande pas de mot de passe.
              </li>
            </ul>
          </div>
          <div className="guide__card">
            <h4>Désactiver (nettoyage complet)</h4>
            <ul>
              <li>Les tâches en cours sont <strong>interrompues</strong> (<code>pkill -u partagpu</code>).</li>
              <li>
                Le compte <code>partagpu</code> est <strong>supprimé</strong>{" "}
                (<code>userdel --remove</code>).
              </li>
              <li>
                Le venv géré est <strong>supprimé</strong> (environ 3 Go libérés).
              </li>
              <li>Le cgroup et les règles de refus SSH et sudo sont également supprimés.</li>
              <li>
                <em>Activer</em> redemande alors une authentification pkexec et
                réinstalle le venv si vous le souhaitez (environ 5 minutes).
              </li>
            </ul>
          </div>
        </div>
        <p>
          Choisissez <em>Pause</em> pour terminer une session de la journée
          en gardant la configuration intacte. Choisissez <em>Désactiver</em>{" "}
          pour libérer la machine de tout résidu PartaGPU.
        </p>
      </section>

      <section className="guide__section">
        <h3>8. Activer le partage sur l'ordinateur d'un absent</h3>
        <ol>
          <li>Allumez l'ordinateur du camarade absent.</li>
          <li>
            Sur l'écran de login (GDM, LightDM…), choisissez l'utilisateur{" "}
            <code>partagpu</code>.
          </li>
          <li>
            Entrez le <strong>mot de passe commun</strong> défini lors de la
            configuration initiale.
          </li>
          <li>PartaGPU se lance automatiquement.</li>
          <li>
            Rejoignez la salle (le code d'accès est dicté par un camarade — ou
            visible sur votre propre poste en maintenant l'icône d'œil).
          </li>
          <li>
            Cliquez sur <em>« Activer le partage »</em> — pas de pkexec demandé,
            le compte est déjà en place.
          </li>
        </ol>
      </section>

      <section className="guide__section">
        <h3>9. Sécurité</h3>
        <ul>
          <li>
            <strong>Authentification HMAC</strong> : chaque requête entre
            pairs porte un en-tête{" "}
            <code>X-PartaGPU-AUTH: &lt;ts_ms&gt;:&lt;HMAC&gt;</code> qui lie
            l'authentification au corps de la requête, ainsi qu'un horodatage
            en millisecondes dans une fenêtre de 30 000 ms (anti-rejeu). Un
            cache des en-têtes déjà vus bloque les rejeux strictement
            identiques (octet par octet). Une autre salle correspond à un
            HMAC différent : la requête est rejetée avant même le
            déchiffrement.
          </li>
          <li>
            <strong>Chiffrement AES-256-GCM</strong> avec{" "}
            <strong>confidentialité persistante</strong> (<em>forward
            secrecy</em>) : la clé de session est dérivée d'un échange
            Diffie-Hellman X25519 éphémère à chaque requête. La clé du
            serveur reste uniquement en RAM et est tournée toutes les
            10 minutes.
          </li>
          <li>
            <strong>Bac à sable <em>bubblewrap</em></strong> par tâche :
            système de fichiers en lecture seule, espace de noms PID isolé,
            réseau coupé par défaut, liste d'autorisation des commandes
            (<em>allowlist</em>), et un sous-cgroup dédié afin qu'une tâche
            ne sature pas la mémoire des autres.
          </li>
          <li>
            <strong>Compte partagpu durci</strong> : SSH bloqué, sudo bloqué,
            et un shell restreint qui ne fait que lancer PartaGPU.
          </li>
          <li>
            <strong>Code d'accès masqué</strong> : la passphrase n'est jamais
            affichée en clair par défaut. Il faut maintenir l'icône d'œil
            pour la révéler.
          </li>
        </ul>
      </section>

      <section className="guide__section guide__section--tip">
        <h3>Bon à savoir</h3>
        <ul>
          <li>
            <strong>Vous n'avez pas de GPU NVIDIA ?</strong> Aucun problème,
            le partage CPU et RAM fonctionne quand même, et la jauge GPU
            reste simplement cachée.
          </li>
          <li>
            <strong>Aucune machine n'est visible ?</strong> Vérifiez que tous
            les postes sont sur le même sous-réseau et que le trafic
            multicast est autorisé (UDP 5353 pour mDNS). Voir{" "}
            <code>docs/TROUBLESHOOTING.md</code> pour le reste des
            diagnostics.
          </li>
          <li>
            <strong>Toolkit ML</strong> (<code>torch</code>,{" "}
            <code>torchvision</code>, <code>scipy</code>, <code>pandas</code>,
            etc.) : elle est installable en un clic depuis{" "}
            <em>« Mon partage »</em> → <em>Environnement Python</em>.
            Comptez environ 3 Go et 5 à 10 minutes de téléchargement. Cela
            évite d'avoir à toucher au Python système.
          </li>
          <li>
            <strong>Documentation détaillée</strong> dans le dépôt :{" "}
            <code>README.md</code>, <code>docs/ARCHITECTURE.md</code>,{" "}
            <code>SECURITY.md</code> et <code>docs/TROUBLESHOOTING.md</code>.
            Des versions anglaises sont également disponibles
            (<code>*.en.md</code>).
          </li>
        </ul>
      </section>
    </div>
  );
}
