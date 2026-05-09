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
          tous les postes qui en font partie peuvent dispatcher du calcul entre
          eux, ceux qui n'en font pas partie sont automatiquement refusés.
        </p>
        <ul>
          <li>
            <strong>Créer une salle</strong> (un seul élève le fait) : bouton{" "}
            <em>« Créer une salle »</em>, choisir un nom (ex.{" "}
            <em>« Salle B204 »</em>). L'app génère un code d'accès de 4 mots
            (<code>pomme-tigre-bleu-ocean</code>). Ce code est affiché masqué
            (<code>*****-*****-****-*****</code>) ; maintenez l'icône d'œil pour
            le révéler le temps de le dicter à voix haute.
          </li>
          <li>
            <strong>Rejoindre une salle</strong> (tous les autres) : bouton{" "}
            <em>« Rejoindre une salle »</em>, entrer le même nom et le code
            de 4 mots dicté.
          </li>
        </ul>
        <p>
          Les pairs vérifiés affichent un badge vert <strong>OK</strong>.
          Ceux d'une autre salle (ou d'aucune) affichent un{" "}
          <strong>?</strong> rouge — vous ne pouvez pas leur dispatcher de
          tâches.
        </p>
      </section>

      <section className="guide__section">
        <h3>2. Activer le partage sur cette machine</h3>
        <p>
          Onglet <strong>« Mon partage »</strong> → bouton <em>« Activer »</em>.
          Une fenêtre PolicyKit demande le mot de passe administrateur. L'app
          crée le compte système <code>partagpu</code>, configure son cgroup
          v2 et ouvre le pare-feu sur les ports nécessaires.
        </p>
        <p>
          Définissez ensuite le <strong>mot de passe du compte partagpu</strong>{" "}
          dans le formulaire qui apparaît. C'est ce mot de passe qui permet de{" "}
          se connecter à la machine depuis l'écran de login (utile pour les
          ordinateurs d'absents — voir plus bas).
        </p>
      </section>

      <section className="guide__section">
        <h3>3. Régler les limites de partage</h3>
        <p>
          Sur chaque jauge de ressource (CPU, RAM, GPU), un{" "}
          <strong>curseur rouge draggable</strong> indique la limite que vous
          partagez. Faites-le glisser à la souris pour ajuster — la modification
          est appliquée instantanément via les cgroups v2 du noyau, sans
          mot de passe.
        </p>
        <p>
          La limite <strong>CPU</strong> est exprimée en % de la{" "}
          <strong>machine entière</strong>, pas d'un cœur : sur un poste
          16 cœurs, 50 % autorise jusqu'à 8 cœurs cumulés pour les tâches
          partagées. La limite <strong>RAM</strong> est en Mo (0 = illimitée).
        </p>
        <p>
          Les jauges affichent en plus la <strong>répartition par utilisateur</strong>
          {" "}: chaque barre est segmentée — un segment vert (<em>« Vous (cette
          machine) »</em>) pour ce que vous consommez en local, puis un segment
          coloré par utilisateur distant qui consomme votre machine via une tâche
          PartaGPU. Survolez un segment pour voir le détail.
        </p>
        <p>
          La jauge GPU peut afficher un avertissement{" "}
          <em>« indicative — CUDA MPS inactif »</em> à côté de la limite : ça
          veut dire que le daemon NVIDIA MPS n'est pas en cours d'exécution
          (typiquement parce que <code>nvidia-cuda-mps-control</code> n'est pas
          installé). Tant que MPS n'est pas actif, le slider GPU est{" "}
          <strong>purement informatif</strong> : il est annoncé aux pairs mais
          le driver CUDA ne le respecte pas, et une tâche peut saturer votre
          GPU à 100 %. Pour activer l'enforcement réel sur Ubuntu/Debian :
        </p>
        <pre className="guide__code">
          <code>sudo apt install nvidia-cuda-toolkit</code>
        </pre>
        <p>
          Puis <em>Mon partage → Désactiver → Activer</em> pour que le helper
          relance le daemon MPS. L'avertissement disparaît alors et le slider
          devient un vrai contrat respecté côté CUDA. Bénéfice annexe sur les
          GPU récents (Ampere/Hopper) : plusieurs tâches CUDA tournent{" "}
          <strong>simultanément</strong> sur des SM différents au lieu de se
          bloquer en time-slicing.
        </p>
        <p>
          Le champ <strong>« Tâches simultanées maximum »</strong> (par défaut
          4) borne le nombre de tâches qui peuvent tourner en même temps. Au-delà,
          les nouvelles arrivantes restent en file d'attente — protection contre
          un pair qui voudrait flooder votre machine.
        </p>
      </section>

      <section className="guide__section">
        <h3>4. Soumettre des tâches</h3>
        <p>
          Onglet <strong>« Mon utilisation »</strong>. Trois façons de
          dispatcher du calcul :
        </p>
        <ul>
          <li>
            <strong>Une commande sur un pair</strong> : formulaire qui prend
            une commande shell ou un script Python uploadé. Idéal pour un test
            ponctuel ou un job non-distribué.
          </li>
          <li>
            <strong>Entraînement DDP multi-machines</strong> : panneau dédié
            qui coche plusieurs pairs et leurs GPU, upload un script PyTorch,
            calcule automatiquement <code>MASTER_ADDR</code>,{" "}
            <code>RANK</code>, <code>WORLD_SIZE</code>,{" "}
            <code>CUDA_VISIBLE_DEVICES</code>, etc. Tableau de progression par
            rang en live, bouton <em>« Tout annuler »</em> qui propage l'arrêt
            sur tous les rangs si un crashe.
          </li>
          <li>
            <strong>Depuis Python</strong> :{" "}
            <code>pip install partagpu</code> puis{" "}
            <code>partagpu.run_remote(...)</code> ou{" "}
            <code>partagpu.distribute("train.py")</code> dans un notebook.
            Trois options utiles : <code>live=True</code> (logs streamés ligne
            par ligne pendant l'entraînement), <code>outputs=["model.pt"]</code>
            {" "}(rapatrie le checkpoint en RAM dans <code>results[0].artifacts</code>),
            et <code>local=False</code> (n'utilise que les pairs distants
            quand vous ne partagez pas votre propre machine). Voir la doc du
            package Python.
          </li>
        </ul>
      </section>

      <section className="guide__section">
        <h3>5. Suivi en temps réel</h3>
        <ul>
          <li>
            <strong>Tableau des tâches</strong> : statut, progression, CPU/RAM/GPU
            par tâche, mis à jour chaque seconde via les événements Tauri (pas
            de polling visible). Trié <strong>plus récent en haut</strong>{" "}
            pour ne pas avoir à scroller après un dispatch.
          </li>
          <li>
            <strong>Durée des tâches</strong> sous la barre de progression :{" "}
            <code>↻ 2m 13s</code> pour une tâche active (rafraîchi à 1 Hz),{" "}
            <code>✓ 6m 42s</code> pour une tâche terminée (figé sur la durée
            totale). Les tâches encore en file d'attente n'ont pas d'horloge.
          </li>
          <li>
            <strong>Bouton Stop</strong> sur chaque tâche en cours : SIGTERM
            propre côté pair, SIGKILL après 2 s. Pour les jobs DDP, l'annulation
            d'un rang propage à tous les autres pour éviter le hang NCCL.
          </li>
          <li>
            <strong>Bouton 🗑 supprimer</strong> sur les tâches terminales
            (Completed/Failed/Cancelled) pour nettoyer l'historique. Refuse si
            la tâche est encore active — vous devez canceller d'abord.
          </li>
          <li>
            <strong>Notifications desktop</strong> : à la fin d'un dispatch
            (Completed / Failed / Cancelled), un toast natif système s'affiche,
            même si l'app n'a pas le focus. Pratique pour s'éloigner pendant
            un long entraînement.
          </li>
          <li>
            <strong>Historique 5 minutes</strong> : sous les jauges de la page{" "}
            <em>« Mon partage »</em>, des sparklines montrent l'évolution
            CPU / RAM / GPU sur les 5 dernières minutes.
          </li>
        </ul>
      </section>

      <section className="guide__section">
        <h3>6. Vue parc</h3>
        <p>
          Onglet <strong>« Vue parc »</strong> : tableau de bord agrégé de
          toutes les machines de la salle. Stats globales en haut (nombre de
          pairs visibles, GPU disponibles, ma conso totale) et une carte par
          pair avec sa capacité offerte et les tâches que vous y exécutez.
          Utile pour un prof qui supervise.
        </p>
      </section>

      <section className="guide__section">
        <h3>7. Pause vs Désactiver</h3>
        <p>Deux niveaux d'arrêt aux sémantiques différentes :</p>
        <div className="guide__comparison">
          <div className="guide__card">
            <h4>Pause (temporaire)</h4>
            <ul>
              <li>Pare-feu fermé, plus de tâches entrantes</li>
              <li>Compte <code>partagpu</code> conservé</li>
              <li>Cgroup conservé</li>
              <li>Venv géré conservé</li>
              <li>
                <em>Reprendre</em> = instantané, pas de mot de passe demandé
              </li>
            </ul>
          </div>
          <div className="guide__card">
            <h4>Désactiver (nettoyage complet)</h4>
            <ul>
              <li>Tâches en cours <strong>tuées</strong> (pkill -u partagpu)</li>
              <li>
                Compte <code>partagpu</code> <strong>supprimé</strong> (userdel
                --remove)
              </li>
              <li>
                Venv géré <strong>supprimé</strong> (~3 Go libérés)
              </li>
              <li>Cgroup et règles SSH/sudo deny supprimés</li>
              <li>
                <em>Activer</em> demande pkexec et re-télécharge le venv si
                voulu (~5 min)
              </li>
            </ul>
          </div>
        </div>
        <p>
          Choisissez <em>Pause</em> pour finir une session du jour. Choisissez{" "}
          <em>Désactiver</em> pour libérer la machine de tout résidu PartaGPU.
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
            <strong>Authentification HMAC</strong> : chaque requête entre pairs
            porte un header <code>X-PartaGPU-AUTH: &lt;ts_ms&gt;:&lt;HMAC&gt;</code>
            {" "}qui lie l'auth au corps + un timestamp en millisecondes dans une
            fenêtre de 30 000 ms (anti-replay), avec un cache des headers déjà
            vus pour bloquer les rejeux byte-identiques. Une autre salle = HMAC
            différent = requête rejetée avant même le déchiffrement.
          </li>
          <li>
            <strong>Chiffrement AES-256-GCM</strong> avec{" "}
            <strong>forward secrecy</strong> : clé de session dérivée d'un
            échange Diffie-Hellman X25519 éphémère par requête. La clé serveur
            est en RAM uniquement et tourne toutes les 10 minutes.
          </li>
          <li>
            <strong>Sandbox bubblewrap</strong> par tâche : système de fichiers
            r/o, namespace PID isolé, network unshare par défaut, allowlist de
            commandes, sous-cgroup dédié pour qu'une tâche n'OOM pas les autres.
          </li>
          <li>
            <strong>Compte partagpu durci</strong> : SSH bloqué, sudo bloqué,
            shell restreint qui ne fait que lancer PartaGPU.
          </li>
          <li>
            <strong>Code d'accès masqué</strong> : la passphrase n'est jamais
            affichée en clair par défaut, il faut maintenir l'icône d'œil pour
            la révéler.
          </li>
        </ul>
      </section>

      <section className="guide__section guide__section--tip">
        <h3>Bon à savoir</h3>
        <ul>
          <li>
            <strong>Pas de GPU NVIDIA</strong> ? Pas grave, le partage CPU + RAM
            fonctionne quand même. La jauge GPU reste cachée.
          </li>
          <li>
            <strong>Aucune machine visible</strong> ? Vérifiez le même
            sous-réseau et le multicast (UDP 5353 pour mDNS). Voir{" "}
            <code>docs/TROUBLESHOOTING.md</code> pour le reste.
          </li>
          <li>
            <strong>Toolkit ML</strong> (torch, torchvision, scipy, pandas,
            etc.) installable en un clic depuis <em>« Mon partage »</em> →{" "}
            <em>Environnement Python</em>. ~3 Go, 5–10 min de download. Évite
            d'avoir à toucher au Python système.
          </li>
          <li>
            <strong>Documentation détaillée</strong> dans le repo :{" "}
            <code>README.md</code>, <code>docs/ARCHITECTURE.md</code>,{" "}
            <code>SECURITY.md</code>, <code>docs/TROUBLESHOOTING.md</code>.
            Versions anglaises dispo (<code>*.en.md</code>).
          </li>
        </ul>
      </section>
    </div>
  );
}
