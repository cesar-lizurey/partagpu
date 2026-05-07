/**
 * Centralised dictionary of UI strings, French + English.
 *
 * The Guide page handles its own translations inline (FR / EN
 * variants of the JSX) because translating prose that long via
 * key-by-sentence would be unreadable. Everything else funnels
 * through `useT()` defined in `i18n.tsx`.
 *
 * Placeholders are written `{name}` and replaced at render time
 * (see `t(key, params)` in `i18n.tsx`).
 */

export type Lang = "fr" | "en";

const fr = {
  // ── Generic / shared ───────────────────────────────────────
  "common.loading": "Chargement…",
  "common.cancel": "Annuler",
  "common.confirm": "Confirmer",
  "common.unknown": "inconnu",
  "common.unlimited": "illimitée",
  "common.none_dash": "—",

  // ── App header / tabs ──────────────────────────────────────
  "app.rename_tooltip": "Cliquez pour renommer cette instance",
  "app.lang_toggle_title": "Switch to English",
  "tabs.sharing": "Mon partage",
  "tabs.usage": "Mon utilisation",
  "tabs.fleet": "Vue parc",
  "tabs.guide": "Guide",

  // ── RoomSetup ──────────────────────────────────────────────
  "room.no_room": "Aucune salle configurée",
  "room.create": "Créer une salle",
  "room.join": "Rejoindre une salle",
  "room.create_placeholder": "Nom de la salle (ex: Salle B204)",
  "room.create_btn": "Créer",
  "room.join_name_placeholder": "Nom de la salle",
  "room.join_pass_placeholder": "Code d'accès (ex: pomme-tigre-bleu-ocean)",
  "room.join_hint": "Demandez le code d'accès au camarade qui a créé la salle.",
  "room.join_btn": "Rejoindre",
  "room.in_room_label": "Salle",
  "room.leave": "Quitter",
  "room.share_hint":
    "Dictez ce code d'accès aux camarades pour qu'ils rejoignent. Maintenez l'icône d'œil pour le révéler.",
  "room.err_need_name": "Entrez un nom de salle.",
  "room.err_need_passphrase": "Entrez le code d'accès.",

  // ── Reveal-on-hold ─────────────────────────────────────────
  "reveal.hold_title": "Maintenez pour afficher",
  "reveal.aria_show": "Maintenir pour afficher la valeur",
  "reveal.aria_hide": "Cacher la valeur",

  // ── MySharing page ─────────────────────────────────────────
  "mysharing.title": "Mon partage",
  "mysharing.subtitle": "Ce que les autres utilisent sur cette machine",
  "mysharing.account_section": "Compte partagpu",
  "mysharing.resources_section": "Ressources de cette machine",
  "mysharing.resources_hint":
    "Faites glisser le curseur rouge sur chaque jauge pour ajuster la limite que vous partagez aux autres.",
  "mysharing.history_title": "Historique des 5 dernières minutes",
  "mysharing.history_sublabel": " — échantillonné toutes les 5 s",
  "mysharing.history_avg": "moy. {value} {unit}",
  "mysharing.history_peak": "pic {value} {unit}",
  "mysharing.history_aria": "Historique {label}",
  "mysharing.breakdown_section": "Répartition par utilisateur",
  "mysharing.python_section": "Environnement Python pour les tâches reçues",
  "mysharing.who_section": "Qui utilise mes ressources ?",
  "mysharing.concurrency_label": "Tâches simultanées maximum :",
  "mysharing.concurrency_hint":
    "Au-delà de cette limite, les tâches reçues attendent leur tour (statut « En attente »). Évite qu'un pair sature votre machine en envoyant 100 tâches d'un coup.",
  "mysharing.cores_suffix": "{n} cœurs",

  // ── User account setup (within MySharing) ──────────────────
  "user.err_too_short": "Le mot de passe doit contenir au moins 4 caractères.",
  "user.err_mismatch": "Les mots de passe ne correspondent pas.",
  "user.status_missing":
    "L'utilisateur partagpu n'existe pas encore. Il sera créé en activant le partage.",
  "user.status_no_login":
    "L'utilisateur partagpu existe mais n'a pas de shell de connexion. Activez le partage pour le mettre à jour.",
  "user.status_no_password":
    "L'utilisateur partagpu existe mais n'a pas de mot de passe. Définissez-en un pour permettre la connexion depuis l'écran de login.",
  "user.status_ready": "Utilisateur partagpu configuré et prêt à l'emploi.",
  "user.status_unknown": "Statut inconnu.",
  "user.hint_modify": "Modifier le mot de passe de l'utilisateur partagpu :",
  "user.hint_define":
    "Définir le mot de passe pour se connecter à cette machine :",
  "user.password_placeholder": "Mot de passe",
  "user.confirm_placeholder": "Confirmer",
  "user.btn_modify": "Modifier",
  "user.btn_define": "Définir",

  // ── ResourceGauge ──────────────────────────────────────────
  "gauge.share_limit": "Limite de partage : ",
  "gauge.input_aria": "Limite de partage {label}",
  "gauge.input_disabled_title": "Activez le partage pour ajuster la limite",
  "gauge.input_drag_title": "Faites glisser pour ajuster la limite de partage",

  // ── SharingToggle ──────────────────────────────────────────
  "sharing.status_disabled": "Désactivé",
  "sharing.status_active": "Actif",
  "sharing.status_paused": "En pause",
  "sharing.btn_enable": "Activer le partage",
  "sharing.btn_pause": "Pause",
  "sharing.btn_resume": "Reprendre",
  "sharing.btn_disable": "Désactiver",
  "sharing.tip_pause":
    "Suspend temporairement les tâches reçues sans rien désinstaller. Cliquez « Reprendre » pour redémarrer instantanément.",
  "sharing.tip_disable":
    "Nettoie complètement PartaGPU : supprime le compte partagpu, tue les tâches, vire le venv géré, ferme le pare-feu. À utiliser pour libérer la machine après usage.",
  "sharing.tip_disable_paused":
    "Nettoie complètement PartaGPU : supprime le compte partagpu, vire le venv géré, ferme le pare-feu.",
  "sharing.confirm_disable":
    "Désactiver le partage va NETTOYER COMPLÈTEMENT PartaGPU sur cette machine :\n\n  • Le compte système 'partagpu' est supprimé\n  • Les tâches en cours sur ce poste sont tuées\n  • Le venv géré (torch + numpy, ~2 Go) est supprimé\n  • Le cgroup et les règles SSH/sudo sont nettoyés\n  • Le pare-feu PartaGPU est fermé\n\nPour ré-utiliser PartaGPU ensuite, il faudra tout re-créer (mot de\npasse administrateur + ré-installer le venv ~5 min).\n\nPour un arrêt temporaire, utilisez plutôt « Pause ».\n\nConfirmer la désactivation complète ?",

  // ── ManagedVenvPanel ───────────────────────────────────────
  "venv.intro":
    "Venv géré pour torch + dépendances data science, monté en bind dans le sandbox des tâches reçues. Voir le Guide pour les détails.",
  "venv.installed": "Installé",
  "venv.not_installed": "Non installé",
  "venv.not_installed_hint_p1":
    "Sans ça, les pairs doivent installer torch + ses dépendances manuellement (",
  "venv.not_installed_hint_p2": ").",
  "venv.btn_install": "Installer la toolkit ML (~3 Go)",
  "venv.btn_install_progress": "Installation… (5 à 10 min)",
  "venv.btn_check_updates": "Vérifier les mises à jour de la toolkit ML",
  "venv.btn_check_updates_progress": "Vérification…",
  "venv.btn_check_updates_title":
    "Relance pip install --upgrade sur la toolkit ML pour récupérer les dernières versions des packages",
  "venv.btn_remove": "Supprimer",
  "venv.btn_remove_progress": "Suppression…",
  "venv.installing_msg":
    "Téléchargement et installation en cours (5 à 10 minutes selon votre connexion). Laissez la fenêtre ouverte — la progression de pip s'affiche ci-dessous en temps réel.",
  "venv.log_summary_one": "Log d'installation ({n} ligne)",
  "venv.log_summary_many": "Log d'installation ({n} lignes)",
  "venv.log_waiting": "(en attente de la première ligne…)",
  "venv.confirm_install":
    "Installer la toolkit ML dans le venv géré ?\n\nPackages : torch, torchvision, numpy, scipy, pandas,\nscikit-learn, matplotlib, pillow.\nTéléchargement de ~3 Go, prend 5 à 10 minutes selon votre\nconnexion. Le mot de passe administrateur sera demandé.",
  "venv.confirm_update":
    "Vérifier les mises à jour de la toolkit ML ?\n\nLance pip install --upgrade sur torch, torchvision, numpy,\nscipy, pandas, scikit-learn, matplotlib, pillow.\nTéléchargement uniquement de ce qui a une nouvelle version.\nLe mot de passe administrateur sera demandé.",
  "venv.confirm_remove":
    "Supprimer le venv géré ?\n\nLes tâches reçues qui utilisaient torch/numpy via ce venv échoueront jusqu'à réinstallation.",

  // ── MyUsage page ───────────────────────────────────────────
  "myusage.title": "Mon utilisation",
  "myusage.subtitle": "Ce que j'utilise sur les autres machines du réseau",
  "myusage.peers_section": "Machines détectées",
  "myusage.peers_hint_p1": "Vous pouvez utiliser les machines à la fois ",
  "myusage.peers_hint_authok": "Auth : OK",
  "myusage.peers_hint_p2": " (dans votre salle) et ",
  "myusage.peers_hint_active": "Partage : Actif",
  "myusage.peers_hint_p3": ". Les machines non vérifiées (Auth : ",
  "myusage.peers_hint_qmark": "?",
  "myusage.peers_hint_p4":
    ") sont dans une autre salle ou aucune — vous ne pourrez pas leur dispatcher de tâches même si elles partagent. Triées : utilisables d'abord, puis le reste.",
  "myusage.command_section": "Lancer une commande sur un pair",
  "myusage.ddp_section": "Entraînement DDP multi-machines",
  "myusage.tasks_section": "Mes tâches en cours",

  // ── PeerTable ──────────────────────────────────────────────
  "peers.empty_default": "Aucune machine détectée sur le réseau.",
  "peers.col_machine": "Machine",
  "peers.col_ip": "IP",
  "peers.col_auth": "Auth",
  "peers.col_sharing": "Partage",
  "peers.col_cpu": "CPU",
  "peers.col_ram": "RAM",
  "peers.col_gpu": "GPU",
  "peers.sharing_active": "Actif",
  "peers.sharing_inactive": "Inactif",
  "peers.badge_conflict_title": "Conflit de hostname — possible usurpation",
  "peers.badge_unverified_title": "Non vérifié",
  "peers.badge_verified_title": "Auth vérifiée (HMAC OK)",
  "peers.conflict_alert_one":
    "Conflit de hostname détecté — 1 machine utilise un nom déjà pris par une autre IP. Cela peut indiquer une tentative d'usurpation d'identité.",
  "peers.conflict_alert_many":
    "Conflit de hostname détecté — {n} machines utilisent un nom déjà pris par une autre IP. Cela peut indiquer une tentative d'usurpation d'identité.",
  "peers.unverified_alert_one":
    "1 machine non vérifiée — les tâches provenant de ce poste seront refusées.",
  "peers.unverified_alert_many":
    "{n} machines non vérifiées — les tâches provenant de ces postes seront refusées.",
  "peers.conflict_icon_title": "Conflit de hostname",

  // ── TaskList / status badges ───────────────────────────────
  "task.status_queued": "En attente",
  "task.status_running": "En cours",
  "task.status_completed": "Terminée",
  "task.status_failed": "Échouée",
  "task.status_cancelled": "Annulée",
  "task.col_command": "Commande",
  "task.col_source": "Source",
  "task.col_target": "Cible",
  "task.col_status": "Statut",
  "task.col_progress": "Progression",
  "task.col_action": "Action",
  "task.empty_incoming": "Personne n'utilise vos ressources actuellement.",
  "task.empty_outgoing": "Vous n'utilisez aucune ressource distante.",
  "task.cancel_btn": "Stop",
  "task.cancel_title": "Arrêter cette tâche",
  "task.cancel_failed": "Annulation refusée : {error}",
  "task.network_badge": "réseau",
  "task.network_badge_title": "Sandbox avec accès réseau (DDP rendezvous)",

  // ── UsageBreakdown ─────────────────────────────────────────
  "breakdown.tasks_one": "({n} tâche)",
  "breakdown.tasks_many": "({n} tâches)",

  // ── TaskDispatcher ─────────────────────────────────────────
  "dispatcher.no_targets":
    "Aucun pair vérifié ne partage de ressources actuellement. Activez le partage côté camarade et vérifiez que vous êtes dans la même salle.",
  "dispatcher.target_label": "Pair cible",
  "dispatcher.timeout_label": "Timeout (s)",
  "dispatcher.what_label": "Quoi exécuter",
  "dispatcher.mode_command": "Une commande",
  "dispatcher.mode_file": "Un fichier uploadé",
  "dispatcher.mode_file_pick_first": "— uploadez un fichier d'abord —",
  "dispatcher.mode_file_choose": "— choisir —",
  "dispatcher.file_args_placeholder": "arguments optionnels (ex: --epochs 10)",
  "dispatcher.file_upload_hint": "Uploadez un fichier dans la section ci-dessous.",
  "dispatcher.workspace_label": "Fichiers à uploader",
  "dispatcher.workspace_add": "Ajouter…",
  "dispatcher.workspace_help_p1":
    "Ces fichiers seront copiés dans le répertoire de travail de la commande sur le pair (par défaut ",
  "dispatcher.workspace_help_p2": "). Référez-y dans la commande par leur nom (ex. ",
  "dispatcher.workspace_help_p3": "). Limite totale : {limit}.",
  "dispatcher.workspace_total": "Total : {size}",
  "dispatcher.workspace_too_big": "(dépasse la limite)",
  "dispatcher.workspace_remove_title": "Retirer ce fichier",
  "dispatcher.network_label": "Autoriser l'accès réseau dans le sandbox du pair",
  "dispatcher.network_help_p1":
    "Par défaut, la tâche tourne sans accès réseau (isolation maximale). Cochez cette case si votre commande a besoin de : ",
  "dispatcher.network_help_p2": "télécharger des données",
  "dispatcher.network_help_p3":
    " (HTTP, HuggingFace…), joindre un autre service du réseau local, ou faire un ",
  "dispatcher.network_help_p4": "entraînement DDP",
  "dispatcher.network_help_p5":
    " (les processus parallèles doivent se synchroniser via le réseau).",
  "dispatcher.btn_launch": "Lancer",
  "dispatcher.btn_launching": "Exécution...",
  "dispatcher.err_no_peer": "Aucun pair sélectionné.",
  "dispatcher.err_empty_cmd": "La commande est vide.",
  "dispatcher.err_workspace_too_big":
    "Le workspace dépasse la limite de {limit}. Total actuel : {total}.",
  "dispatcher.err_read_files": "Échec de lecture des fichiers : {error}",
  "dispatcher.result_target": "cible : ",
  "dispatcher.result_exit": " · exit code : ",
  "dispatcher.result_live": " · sortie en direct…",
  "dispatcher.result_no_output": "(aucune sortie)",
  "dispatcher.result_waiting":
    "En attente de la première ligne de sortie…",
  "dispatcher.stdout_summary": "stdout ({n} car.)",
  "dispatcher.stderr_summary": "stderr ({n} car.)",
  "dispatcher.peer_option": "{name} ({ip}) — {gpus} GPU",

  // ── DDPDispatcher ──────────────────────────────────────────
  "ddp.no_targets":
    "Aucun pair vérifié n'expose de GPU pour le moment. Activez le partage côté camarade et vérifiez que vous êtes dans la même salle.",
  "ddp.intro_p1":
    "Lance un script Python en mode DDP (un processus par GPU sélectionné, rendez-vous NCCL/Gloo sur le LAN). Le script et ses dépendances sont envoyés à chaque pair ; les variables d'environnement ",
  "ddp.intro_p2": ", ",
  "ddp.intro_p3": " sont positionnées automatiquement.",
  "ddp.targets_legend": "Cibles ({n} GPU sélectionnés)",
  "ddp.peer_max_title": "max {n} GPU",
  "ddp.backend_label": "Backend",
  "ddp.backend_nccl": "NCCL (GPU)",
  "ddp.backend_gloo": "Gloo (CPU/GPU)",
  "ddp.master_port_label": "Port maître",
  "ddp.timeout_label": "Timeout (s)",
  "ddp.script_label": "Script Python",
  "ddp.script_pick": "Choisir…",
  "ddp.script_none": "— aucun —",
  "ddp.script_args_label": "Arguments du script",
  "ddp.extras_label": "Fichiers compagnons",
  "ddp.extras_add": "Ajouter…",
  "ddp.extras_limit_hint":
    "Limite totale (script + compagnons) : {limit}.",
  "ddp.btn_launch": "Lancer ({n} ranks)",
  "ddp.btn_launching": "Lancement… ({n} ranks)",
  "ddp.btn_cancel_all": "Tout annuler",
  "ddp.ranks_title": "Ranks",
  "ddp.col_rank": "Rank",
  "ddp.col_peer": "Pair",
  "ddp.col_gpu": "GPU",
  "ddp.col_state": "État",
  "ddp.col_progress": "Progression",
  "ddp.dev_label": "dev {n}",
  "ddp.err_no_script": "Sélectionnez un script Python à exécuter.",
  "ddp.err_no_gpu": "Sélectionnez au moins un GPU sur un pair.",
  "ddp.err_workspace_too_big":
    "Workspace trop volumineux ({size} / {limit}).",
  "ddp.err_read_files": "Échec de lecture des fichiers : {error}",
  "ddp.abort_with_target":
    "Rank {rank} ({peer}) a échoué : {reason}. Annulation des autres ranks…",
  "ddp.abort_no_target":
    "Rank {rank} a échoué : {reason}. Annulation des autres ranks…",
  "ddp.errors": "Erreurs : {list}",

  // ── SecurityLog ────────────────────────────────────────────
  "seclog.title": "Journal de sécurité",
  "seclog.show": "Afficher",
  "seclog.hide": "Masquer",
  "seclog.empty": "Aucun événement enregistré.",
  "seclog.clear": "Effacer le journal",
  "seclog.level_info": "INFO",
  "seclog.level_warning": "WARN",
  "seclog.level_alert": "ALERTE",

  // ── Fleet page ─────────────────────────────────────────────
  "fleet.title": "Vue parc",
  "fleet.subtitle": "État global de toutes les machines de la salle",
  "fleet.stat_visible": "Pairs visibles",
  "fleet.stat_usable": "Pairs utilisables",
  "fleet.stat_usable_hint": "Auth OK + Partage Actif",
  "fleet.stat_gpus": "GPU dans la salle",
  "fleet.stat_gpus_hint": "Somme sur les pairs utilisables",
  "fleet.stat_my_tasks": "Mes tâches actives",
  "fleet.stat_cpu_total": "CPU total (mes tâches)",
  "fleet.stat_ram_total": "RAM totale",
  "fleet.stat_gpu_total": "GPU total",
  "fleet.empty":
    "Aucun pair détecté pour l'instant. Vérifiez que d'autres machines tournent PartaGPU sur le même sous-réseau et qu'elles ont rejoint la même salle.",
  "fleet.note_p1": "Cette vue montre ce que ",
  "fleet.note_p2": "vous",
  "fleet.note_p3":
    " exécutez sur chaque pair. Pour savoir ce que d'autres classmates dispatchent sur eux, une route ",
  "fleet.note_p4": " agrégée serait nécessaire — cf. TODO.md.",
  "fleet.peer_auth_label": "Auth : ",
  "fleet.peer_auth_ok": "OK",
  "fleet.peer_auth_unknown": "?",
  "fleet.peer_sharing_label": "Partage : ",
  "fleet.peer_sharing_active": "Actif",
  "fleet.peer_sharing_inactive": "—",
  "fleet.peer_conflict": "conflit hostname",
  "fleet.peer_gpu_limit": "limite {n} %",
  "fleet.peer_my_tasks": "Mes tâches ici : {n}",
  "fleet.peer_available": "Disponible",
  "fleet.peer_unavailable": "Indisponible",

  // ── Notifications (desktop toasts) ─────────────────────────
  "notify.completed": "✅ Tâche terminée",
  "notify.failed": "❌ Tâche échouée",
  "notify.failed_with_exit": "❌ Tâche échouée (exit {code})",
  "notify.cancelled": "⏹ Tâche annulée",
} as const;

const en: Record<keyof typeof fr, string> = {
  // ── Generic / shared ───────────────────────────────────────
  "common.loading": "Loading…",
  "common.cancel": "Cancel",
  "common.confirm": "Confirm",
  "common.unknown": "unknown",
  "common.unlimited": "unlimited",
  "common.none_dash": "—",

  // ── App header / tabs ──────────────────────────────────────
  "app.rename_tooltip": "Click to rename this instance",
  "app.lang_toggle_title": "Passer en français",
  "tabs.sharing": "My sharing",
  "tabs.usage": "My usage",
  "tabs.fleet": "Fleet view",
  "tabs.guide": "Guide",

  // ── RoomSetup ──────────────────────────────────────────────
  "room.no_room": "No room configured",
  "room.create": "Create a room",
  "room.join": "Join a room",
  "room.create_placeholder": "Room name (e.g. Room B204)",
  "room.create_btn": "Create",
  "room.join_name_placeholder": "Room name",
  "room.join_pass_placeholder": "Access code (e.g. apple-tiger-blue-ocean)",
  "room.join_hint": "Ask the classmate who created the room for the access code.",
  "room.join_btn": "Join",
  "room.in_room_label": "Room",
  "room.leave": "Leave",
  "room.share_hint":
    "Read this access code aloud so classmates can join. Hold the eye icon to reveal it.",
  "room.err_need_name": "Enter a room name.",
  "room.err_need_passphrase": "Enter the access code.",

  // ── Reveal-on-hold ─────────────────────────────────────────
  "reveal.hold_title": "Hold to reveal",
  "reveal.aria_show": "Hold to reveal the value",
  "reveal.aria_hide": "Hide the value",

  // ── MySharing page ─────────────────────────────────────────
  "mysharing.title": "My sharing",
  "mysharing.subtitle": "What others are using on this machine",
  "mysharing.account_section": "partagpu account",
  "mysharing.resources_section": "Resources on this machine",
  "mysharing.resources_hint":
    "Drag the red cursor on each gauge to adjust the limit you share with others.",
  "mysharing.history_title": "Last 5 minutes",
  "mysharing.history_sublabel": " — sampled every 5 s",
  "mysharing.history_avg": "avg {value} {unit}",
  "mysharing.history_peak": "peak {value} {unit}",
  "mysharing.history_aria": "{label} history",
  "mysharing.breakdown_section": "Breakdown by user",
  "mysharing.python_section": "Python environment for incoming tasks",
  "mysharing.who_section": "Who is using my resources?",
  "mysharing.concurrency_label": "Max concurrent tasks:",
  "mysharing.concurrency_hint":
    "Beyond this limit, incoming tasks wait their turn (Queued status). Prevents a peer from saturating your machine by sending 100 tasks at once.",
  "mysharing.cores_suffix": "{n} cores",

  // ── User account setup ────────────────────────────────────
  "user.err_too_short": "Password must be at least 4 characters.",
  "user.err_mismatch": "Passwords don't match.",
  "user.status_missing":
    "The partagpu user does not exist yet. It will be created when sharing is enabled.",
  "user.status_no_login":
    "The partagpu user exists but has no login shell. Enable sharing to update it.",
  "user.status_no_password":
    "The partagpu user exists but has no password. Set one to allow login from the system login screen.",
  "user.status_ready": "partagpu user configured and ready.",
  "user.status_unknown": "Unknown status.",
  "user.hint_modify": "Change the partagpu user's password:",
  "user.hint_define": "Set the password to log in to this machine:",
  "user.password_placeholder": "Password",
  "user.confirm_placeholder": "Confirm",
  "user.btn_modify": "Change",
  "user.btn_define": "Set",

  // ── ResourceGauge ──────────────────────────────────────────
  "gauge.share_limit": "Share limit: ",
  "gauge.input_aria": "{label} share limit",
  "gauge.input_disabled_title": "Enable sharing to adjust the limit",
  "gauge.input_drag_title": "Drag to adjust the share limit",

  // ── SharingToggle ──────────────────────────────────────────
  "sharing.status_disabled": "Disabled",
  "sharing.status_active": "Active",
  "sharing.status_paused": "Paused",
  "sharing.btn_enable": "Enable sharing",
  "sharing.btn_pause": "Pause",
  "sharing.btn_resume": "Resume",
  "sharing.btn_disable": "Disable",
  "sharing.tip_pause":
    "Temporarily suspend incoming tasks without uninstalling anything. Click \"Resume\" to restart instantly.",
  "sharing.tip_disable":
    "Fully clean up PartaGPU: delete the partagpu account, kill running tasks, remove the managed venv, close the firewall. Use this to fully release the machine.",
  "sharing.tip_disable_paused":
    "Fully clean up PartaGPU: delete the partagpu account, remove the managed venv, close the firewall.",
  "sharing.confirm_disable":
    "Disabling sharing will FULLY CLEAN UP PartaGPU on this machine:\n\n  • The 'partagpu' system account is deleted\n  • Running tasks on this host are killed\n  • The managed venv (torch + numpy, ~2 GB) is removed\n  • The cgroup and SSH/sudo rules are cleaned up\n  • The PartaGPU firewall is closed\n\nTo use PartaGPU again afterwards, everything will need to be re-created\n(admin password + re-install the venv ~5 min).\n\nFor a temporary pause, use \"Pause\" instead.\n\nConfirm full deactivation?",

  // ── ManagedVenvPanel ───────────────────────────────────────
  "venv.intro":
    "Managed venv for torch + data science dependencies, bind-mounted into the sandbox of incoming tasks. See the Guide for details.",
  "venv.installed": "Installed",
  "venv.not_installed": "Not installed",
  "venv.not_installed_hint_p1":
    "Without it, peers must install torch + its dependencies manually (",
  "venv.not_installed_hint_p2": ").",
  "venv.btn_install": "Install the ML toolkit (~3 GB)",
  "venv.btn_install_progress": "Installing… (5 to 10 min)",
  "venv.btn_check_updates": "Check for ML toolkit updates",
  "venv.btn_check_updates_progress": "Checking…",
  "venv.btn_check_updates_title":
    "Re-runs pip install --upgrade on the ML toolkit to get the latest package versions",
  "venv.btn_remove": "Remove",
  "venv.btn_remove_progress": "Removing…",
  "venv.installing_msg":
    "Downloading and installing (5 to 10 minutes depending on your connection). Keep the window open — pip's progress is shown live below.",
  "venv.log_summary_one": "Install log ({n} line)",
  "venv.log_summary_many": "Install log ({n} lines)",
  "venv.log_waiting": "(waiting for the first line…)",
  "venv.confirm_install":
    "Install the ML toolkit in the managed venv?\n\nPackages: torch, torchvision, numpy, scipy, pandas,\nscikit-learn, matplotlib, pillow.\nDownloads ~3 GB, takes 5 to 10 minutes depending on your\nconnection. The admin password will be requested.",
  "venv.confirm_update":
    "Check for ML toolkit updates?\n\nRuns pip install --upgrade on torch, torchvision, numpy,\nscipy, pandas, scikit-learn, matplotlib, pillow.\nOnly downloads packages with a new version.\nThe admin password will be requested.",
  "venv.confirm_remove":
    "Remove the managed venv?\n\nIncoming tasks that used torch/numpy via this venv will fail until reinstall.",

  // ── MyUsage page ───────────────────────────────────────────
  "myusage.title": "My usage",
  "myusage.subtitle": "What I'm using on other machines on the network",
  "myusage.peers_section": "Detected machines",
  "myusage.peers_hint_p1": "You can use machines that are both ",
  "myusage.peers_hint_authok": "Auth: OK",
  "myusage.peers_hint_p2": " (in your room) and ",
  "myusage.peers_hint_active": "Sharing: Active",
  "myusage.peers_hint_p3": ". Unverified machines (Auth: ",
  "myusage.peers_hint_qmark": "?",
  "myusage.peers_hint_p4":
    ") are in another room or none — you cannot dispatch tasks to them even if they share. Sorted: usable first, then the rest.",
  "myusage.command_section": "Run a command on a peer",
  "myusage.ddp_section": "Multi-machine DDP training",
  "myusage.tasks_section": "My running tasks",

  // ── PeerTable ──────────────────────────────────────────────
  "peers.empty_default": "No machine detected on the network.",
  "peers.col_machine": "Machine",
  "peers.col_ip": "IP",
  "peers.col_auth": "Auth",
  "peers.col_sharing": "Sharing",
  "peers.col_cpu": "CPU",
  "peers.col_ram": "RAM",
  "peers.col_gpu": "GPU",
  "peers.sharing_active": "Active",
  "peers.sharing_inactive": "Inactive",
  "peers.badge_conflict_title": "Hostname conflict — possible spoofing",
  "peers.badge_unverified_title": "Not verified",
  "peers.badge_verified_title": "Auth verified (HMAC OK)",
  "peers.conflict_alert_one":
    "Hostname conflict detected — 1 machine uses a name already taken by a different IP. This may indicate a spoofing attempt.",
  "peers.conflict_alert_many":
    "Hostname conflict detected — {n} machines use a name already taken by a different IP. This may indicate a spoofing attempt.",
  "peers.unverified_alert_one":
    "1 unverified machine — tasks coming from this host will be rejected.",
  "peers.unverified_alert_many":
    "{n} unverified machines — tasks coming from these hosts will be rejected.",
  "peers.conflict_icon_title": "Hostname conflict",

  // ── TaskList / status badges ───────────────────────────────
  "task.status_queued": "Queued",
  "task.status_running": "Running",
  "task.status_completed": "Completed",
  "task.status_failed": "Failed",
  "task.status_cancelled": "Cancelled",
  "task.col_command": "Command",
  "task.col_source": "Source",
  "task.col_target": "Target",
  "task.col_status": "Status",
  "task.col_progress": "Progress",
  "task.col_action": "Action",
  "task.empty_incoming": "Nobody is using your resources right now.",
  "task.empty_outgoing": "You aren't using any remote resource.",
  "task.cancel_btn": "Stop",
  "task.cancel_title": "Stop this task",
  "task.cancel_failed": "Cancel rejected: {error}",
  "task.network_badge": "network",
  "task.network_badge_title": "Sandbox with network access (DDP rendezvous)",

  // ── UsageBreakdown ─────────────────────────────────────────
  "breakdown.tasks_one": "({n} task)",
  "breakdown.tasks_many": "({n} tasks)",

  // ── TaskDispatcher ─────────────────────────────────────────
  "dispatcher.no_targets":
    "No verified peer is sharing resources right now. Ask your classmate to enable sharing and check that you're in the same room.",
  "dispatcher.target_label": "Target peer",
  "dispatcher.timeout_label": "Timeout (s)",
  "dispatcher.what_label": "What to run",
  "dispatcher.mode_command": "A command",
  "dispatcher.mode_file": "An uploaded file",
  "dispatcher.mode_file_pick_first": "— upload a file first —",
  "dispatcher.mode_file_choose": "— pick —",
  "dispatcher.file_args_placeholder": "optional arguments (e.g. --epochs 10)",
  "dispatcher.file_upload_hint": "Upload a file in the section below.",
  "dispatcher.workspace_label": "Files to upload",
  "dispatcher.workspace_add": "Add…",
  "dispatcher.workspace_help_p1":
    "These files are copied into the command's working directory on the peer (default ",
  "dispatcher.workspace_help_p2": "). Reference them by name in the command (e.g. ",
  "dispatcher.workspace_help_p3": "). Total limit: {limit}.",
  "dispatcher.workspace_total": "Total: {size}",
  "dispatcher.workspace_too_big": "(over the limit)",
  "dispatcher.workspace_remove_title": "Remove this file",
  "dispatcher.network_label": "Allow network access in the peer's sandbox",
  "dispatcher.network_help_p1":
    "By default, the task runs without network access (maximum isolation). Tick this box if your command needs to: ",
  "dispatcher.network_help_p2": "download data",
  "dispatcher.network_help_p3":
    " (HTTP, HuggingFace…), reach another local-network service, or run a ",
  "dispatcher.network_help_p4": "DDP training",
  "dispatcher.network_help_p5":
    " (parallel processes must synchronize over the network).",
  "dispatcher.btn_launch": "Launch",
  "dispatcher.btn_launching": "Running...",
  "dispatcher.err_no_peer": "No peer selected.",
  "dispatcher.err_empty_cmd": "The command is empty.",
  "dispatcher.err_workspace_too_big":
    "Workspace exceeds the {limit} limit. Current total: {total}.",
  "dispatcher.err_read_files": "Failed to read files: {error}",
  "dispatcher.result_target": "target: ",
  "dispatcher.result_exit": " · exit code: ",
  "dispatcher.result_live": " · live output…",
  "dispatcher.result_no_output": "(no output)",
  "dispatcher.result_waiting": "Waiting for the first line of output…",
  "dispatcher.stdout_summary": "stdout ({n} chars)",
  "dispatcher.stderr_summary": "stderr ({n} chars)",
  "dispatcher.peer_option": "{name} ({ip}) — {gpus} GPU",

  // ── DDPDispatcher ──────────────────────────────────────────
  "ddp.no_targets":
    "No verified peer exposes a GPU right now. Ask your classmate to enable sharing and check that you're in the same room.",
  "ddp.intro_p1":
    "Run a Python script in DDP mode (one process per selected GPU, NCCL/Gloo rendezvous over LAN). The script and its dependencies are sent to each peer; the environment variables ",
  "ddp.intro_p2": ", ",
  "ddp.intro_p3": " are set automatically.",
  "ddp.targets_legend": "Targets ({n} GPUs selected)",
  "ddp.peer_max_title": "max {n} GPU",
  "ddp.backend_label": "Backend",
  "ddp.backend_nccl": "NCCL (GPU)",
  "ddp.backend_gloo": "Gloo (CPU/GPU)",
  "ddp.master_port_label": "Master port",
  "ddp.timeout_label": "Timeout (s)",
  "ddp.script_label": "Python script",
  "ddp.script_pick": "Pick…",
  "ddp.script_none": "— none —",
  "ddp.script_args_label": "Script arguments",
  "ddp.extras_label": "Companion files",
  "ddp.extras_add": "Add…",
  "ddp.extras_limit_hint":
    "Total limit (script + companions): {limit}.",
  "ddp.btn_launch": "Launch ({n} ranks)",
  "ddp.btn_launching": "Launching… ({n} ranks)",
  "ddp.btn_cancel_all": "Cancel all",
  "ddp.ranks_title": "Ranks",
  "ddp.col_rank": "Rank",
  "ddp.col_peer": "Peer",
  "ddp.col_gpu": "GPU",
  "ddp.col_state": "State",
  "ddp.col_progress": "Progress",
  "ddp.dev_label": "dev {n}",
  "ddp.err_no_script": "Pick a Python script to run.",
  "ddp.err_no_gpu": "Select at least one GPU on a peer.",
  "ddp.err_workspace_too_big": "Workspace too large ({size} / {limit}).",
  "ddp.err_read_files": "Failed to read files: {error}",
  "ddp.abort_with_target":
    "Rank {rank} ({peer}) failed: {reason}. Cancelling other ranks…",
  "ddp.abort_no_target":
    "Rank {rank} failed: {reason}. Cancelling other ranks…",
  "ddp.errors": "Errors: {list}",

  // ── SecurityLog ────────────────────────────────────────────
  "seclog.title": "Security log",
  "seclog.show": "Show",
  "seclog.hide": "Hide",
  "seclog.empty": "No event recorded.",
  "seclog.clear": "Clear log",
  "seclog.level_info": "INFO",
  "seclog.level_warning": "WARN",
  "seclog.level_alert": "ALERT",

  // ── Fleet page ─────────────────────────────────────────────
  "fleet.title": "Fleet view",
  "fleet.subtitle": "Global state of every machine in the room",
  "fleet.stat_visible": "Visible peers",
  "fleet.stat_usable": "Usable peers",
  "fleet.stat_usable_hint": "Auth OK + Sharing Active",
  "fleet.stat_gpus": "GPUs in the room",
  "fleet.stat_gpus_hint": "Sum across usable peers",
  "fleet.stat_my_tasks": "My active tasks",
  "fleet.stat_cpu_total": "Total CPU (my tasks)",
  "fleet.stat_ram_total": "Total RAM",
  "fleet.stat_gpu_total": "Total GPU",
  "fleet.empty":
    "No peer detected yet. Make sure other machines are running PartaGPU on the same subnet and have joined the same room.",
  "fleet.note_p1": "This view shows what ",
  "fleet.note_p2": "you",
  "fleet.note_p3":
    " are running on each peer. To see what other classmates dispatch on them, an aggregated ",
  "fleet.note_p4": " route would be needed — see TODO.md.",
  "fleet.peer_auth_label": "Auth: ",
  "fleet.peer_auth_ok": "OK",
  "fleet.peer_auth_unknown": "?",
  "fleet.peer_sharing_label": "Sharing: ",
  "fleet.peer_sharing_active": "Active",
  "fleet.peer_sharing_inactive": "—",
  "fleet.peer_conflict": "hostname conflict",
  "fleet.peer_gpu_limit": "limit {n} %",
  "fleet.peer_my_tasks": "My tasks here: {n}",
  "fleet.peer_available": "Available",
  "fleet.peer_unavailable": "Unavailable",

  // ── Notifications (desktop toasts) ─────────────────────────
  "notify.completed": "✅ Task completed",
  "notify.failed": "❌ Task failed",
  "notify.failed_with_exit": "❌ Task failed (exit {code})",
  "notify.cancelled": "⏹ Task cancelled",
};

export const messages = { fr, en };

export type MessageKey = keyof typeof fr;
