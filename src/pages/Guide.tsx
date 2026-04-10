export function Guide() {
  return (
    <div className="page guide">
      <h2>Guide — Comment ça marche ?</h2>

      <section className="guide__section">
        <h3>1. Activer le partage</h3>
        <p>
          Rendez-vous dans l'onglet <strong>« Mon partage »</strong> et cliquez
          sur <strong>« Activer le partage »</strong>. Un utilisateur{" "}
          <code>partagpu</code> sera créé automatiquement sur la machine. Cette
          opération nécessite le mot de passe administrateur (via une fenêtre
          native).
        </p>
      </section>

      <section className="guide__section">
        <h3>2. Définir le mot de passe du compte partagpu</h3>
        <p>
          Après l'activation, un formulaire vous invite à{" "}
          <strong>définir un mot de passe</strong> pour le compte{" "}
          <code>partagpu</code>. Ce mot de passe permet de{" "}
          <strong>se connecter à la machine depuis l'écran de login</strong>{" "}
          (GDM, LightDM, etc.).
        </p>
        <p>
          <strong>Cas d'usage principal :</strong> un camarade est absent et a
          éteint son ordinateur. Vous pouvez l'allumer, choisir l'utilisateur{" "}
          <code>partagpu</code> sur l'écran de connexion, entrer le mot de passe
          convenu, et activer le partage de sa machine. PartaGPU se lance
          automatiquement à la connexion.
        </p>
      </section>

      <section className="guide__section">
        <h3>3. Régler les limites</h3>
        <p>
          Utilisez les <strong>sliders</strong> pour définir le pourcentage
          maximum de CPU, RAM et GPU à partager. Ces limites sont appliquées via
          les <strong>cgroups v2</strong> du noyau Linux.
        </p>
        <p>
          Vous pouvez modifier ces limites à tout moment, même pendant
          l'exécution de tâches.
        </p>
      </section>

      <section className="guide__section">
        <h3>4. Découverte automatique</h3>
        <p>
          Les machines du réseau local se découvrent automatiquement via{" "}
          <strong>mDNS</strong> (Multicast DNS). Dès qu'un poste active le
          partage, il apparaît dans l'onglet <strong>« Mon utilisation »</strong>{" "}
          de toutes les autres machines.
        </p>
        <p>
          Aucune configuration réseau manuelle n'est nécessaire — il suffit
          d'être sur le même réseau local.
        </p>
      </section>

      <section className="guide__section">
        <h3>5. Soumettre une tâche</h3>
        <p>
          Dans l'onglet <strong>« Mon utilisation »</strong>, vous pouvez voir
          les machines disponibles et leur capacité. Pour soumettre une tâche de
          calcul, sélectionnez une machine cible et envoyez votre commande ou
          script.
        </p>
        <p>
          La progression s'affiche en temps réel dans le tableau des tâches.
        </p>
      </section>

      <section className="guide__section">
        <h3>6. Pause et reprise</h3>
        <p>
          Le bouton <strong>« Pause »</strong> suspend temporairement le partage
          sans supprimer la configuration. Le bouton{" "}
          <strong>« Désactiver »</strong> arrête complètement le partage.
        </p>
        <p>
          Le compte <code>partagpu</code> et son mot de passe{" "}
          <strong>persistent</strong> même après la désactivation — il n'y a pas
          besoin de tout reconfigurer à chaque fois.
        </p>
      </section>

      <section className="guide__section">
        <h3>7. Comprendre les deux onglets</h3>
        <div className="guide__comparison">
          <div className="guide__card">
            <h4>Mon partage</h4>
            <p>
              Ce que <strong>les autres</strong> utilisent{" "}
              <strong>chez moi</strong>.
            </p>
            <ul>
              <li>Voir qui consomme mes ressources</li>
              <li>Combien de CPU/RAM/GPU est utilisé</li>
              <li>Régler mes limites de partage</li>
              <li>Gérer le compte partagpu et son mot de passe</li>
            </ul>
          </div>
          <div className="guide__card">
            <h4>Mon utilisation</h4>
            <p>
              Ce que <strong>j'utilise</strong> chez{" "}
              <strong>les autres</strong>.
            </p>
            <ul>
              <li>Voir les machines disponibles sur le réseau</li>
              <li>Suivre mes tâches envoyées</li>
              <li>Voir la progression en temps réel</li>
              <li>Soumettre de nouvelles tâches</li>
            </ul>
          </div>
        </div>
      </section>

      <section className="guide__section">
        <h3>8. Sécurité</h3>
        <ul>
          <li>
            Le compte <code>partagpu</code> est un compte{" "}
            <strong>dédié et isolé</strong> — il ne peut pas accéder aux
            fichiers des autres utilisateurs de la machine.
          </li>
          <li>
            Les tâches sont confinées dans un <strong>cgroup</strong> avec des
            limites strictes de CPU, RAM et GPU.
          </li>
          <li>
            Les communications entre machines sont chiffrées (
            <strong>TLS mutuel</strong>).
          </li>
          <li>
            Chaque machine garde le <strong>contrôle total</strong> sur ce
            qu'elle partage et peut couper le partage à tout moment.
          </li>
        </ul>
      </section>

      <section className="guide__section guide__section--tip">
        <h3>Bon à savoir</h3>
        <ul>
          <li>
            <strong>Ordinateur d'un absent :</strong> allumez la machine,
            connectez-vous avec le compte <code>partagpu</code> et le mot de
            passe défini. PartaGPU se lance automatiquement.
          </li>
          <li>
            Si vous ne voyez pas d'autres machines, vérifiez que vous êtes sur
            le même réseau local et que le pare-feu autorise le multicast (port
            5353 UDP pour mDNS).
          </li>
          <li>
            Un GPU NVIDIA avec les drivers installés est nécessaire pour le
            partage GPU. Sans GPU, seuls le CPU et la RAM sont disponibles.
          </li>
          <li>
            Les ressources sont rafraîchies toutes les 3 secondes.
          </li>
        </ul>
      </section>
    </div>
  );
}
