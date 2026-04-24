use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use totp_rs::{Algorithm, Secret, TOTP};

const TOTP_STEP: u64 = 30;
const TOTP_DIGITS: usize = 6;
const CONFIG_DIR: &str = "partagpu";
const ROOM_FILE: &str = "room.json";
/// 256 common French words — short, easy to spell, easy to dictate.
/// 4 words = 256^4 = ~4 billion combinations.
const WORDLIST: &[&str; 256] = &[
    "abri","acier","aigle","aimer","algue","aller","ambre","amour",
    "ancre","ange","arbre","arche","astre","atlas","avion","azote",
    "badge","balai","barbe","barre","beton","bijou","blanc","blaze",
    "boeuf","boire","bombe","bonne","bosse","bravo","brise","brume",
    "cable","cacao","cadre","calme","canal","cargo","cedre","celer",
    "champ","chose","cible","cidre","clair","clown","cobra","coeur",
    "conte","coude","crabe","crane","cycle","dalle","danse","delta",
    "digue","dorer","douze","dragon","droit","duvet","ecart","echec",
    "ecran","effet","eleve","email","envoi","epice","etage","etoile",
    "etude","exact","exode","facile","faune","ferme","fibre","figue",
    "final","fleur","force","forme","foule","frais","fruit","fumee",
    "galet","garce","geler","genou","givre","globe","gomme","goyave",
    "grain","grise","guide","habit","herbe","heure","hiver","huile",
    "icone","image","index","infra","issue","ivoire","jadis","jeton",
    "joker","jouet","juice","jurer","karma","kayak","koala","label",
    "lacet","laine","lampe","lance","large","laser","laver","lever",
    "libre","ligue","lilas","linge","liste","livre","loche","lotus",
    "loupe","lueur","lundi","macle","magen","mains","major","marge",
    "masse","melon","merle","micro","mille","mitre","modem","moine",
    "monde","morse","moule","muret","nappe","neige","niche","noble",
    "noeud","nuage","ocean","olive","ombre","ongle","opale","orage",
    "oscar","otage","ovale","ozone","pagne","panda","panne","parer",
    "patio","pause","peage","perle","phase","piece","piste","pixel",
    "place","plage","plomb","pluie","pneu","poele","point","pomme",
    "porte","poste","prime","prune","pulse","quand","radar","radis",
    "rampe","ravin","rebut","regle","reine","repos","riche","rival",
    "roche","roman","rotin","rouge","ruban","sable","sabot","sapin",
    "sauce","sauge","selle","seuil","siege","signe","socle","sonar",
    "souci","spore","stage","style","sucre","table","talon","tamis",
    "tasse","temps","tigre","tonne","trace","train","trial","tribu",
    "tulip","turbo","ultra","union","urine","utile","vague","valse",
    "vaste","verre","vigne","ville","vitre","voile","volte","wagon",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomStatus {
    pub joined: bool,
    pub room_name: String,
    pub passphrase: String,
    pub current_code: String,
    pub seconds_remaining: u64,
}

#[derive(Clone)]
pub struct AuthManager {
    state: Arc<Mutex<Option<RoomState>>>,
}

struct RoomState {
    room_name: String,
    totp: TOTP,
    secret_base32: String,
    passphrase: String,
}

/// Persisted room data (saved to ~/.config/partagpu/room.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedRoom {
    room_name: String,
    secret_base32: String,
}

fn config_path() -> PathBuf {
    let base = dirs_next().unwrap_or_else(|| PathBuf::from("."));
    base.join(CONFIG_DIR).join(ROOM_FILE)
}

/// Get the user config directory (~/.config or $XDG_CONFIG_HOME).
fn dirs_next() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
        })
}

fn save_room(room_name: &str, secret_base32: &str) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let data = SavedRoom {
        room_name: room_name.to_string(),
        secret_base32: secret_base32.to_string(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&data) {
        let _ = fs::write(&path, json);
    }
}

fn load_room() -> Option<SavedRoom> {
    let path = config_path();
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

fn delete_room_file() {
    let path = config_path();
    let _ = fs::remove_file(&path);
}

impl AuthManager {
    pub fn new() -> Self {
        let mgr = Self {
            state: Arc::new(Mutex::new(None)),
        };

        // Restore saved room from disk
        if let Some(saved) = load_room() {
            if let Ok(totp) = build_totp(&saved.secret_base32, &saved.room_name) {
                let passphrase = secret_to_passphrase(&saved.secret_base32);
                *mgr.state.lock().unwrap() = Some(RoomState {
                    room_name: saved.room_name,
                    totp,
                    secret_base32: saved.secret_base32,
                    passphrase,
                });
            }
        }

        mgr
    }

    /// Create a new room: generate a random 4-word passphrase, then derive
    /// the TOTP secret from it (same path as join_room) so both sides match.
    pub fn create_room(&self, room_name: &str) -> Result<CreateRoomOutput, String> {
        // Generate 4 random bytes → 4-word passphrase
        let secret = Secret::generate_secret();
        let raw_b32 = secret.to_encoded().to_string();
        let passphrase = secret_to_passphrase(&raw_b32);

        // Derive the canonical secret from the passphrase (same as a joiner would)
        let secret_b32 = passphrase_to_secret(&passphrase)?;

        let totp = build_totp(&secret_b32, room_name)?;

        let mut state = self.state.lock().unwrap();
        *state = Some(RoomState {
            room_name: room_name.to_string(),
            totp,
            secret_base32: secret_b32.clone(),
            passphrase: passphrase.clone(),
        });

        save_room(room_name, &secret_b32);

        Ok(CreateRoomOutput {
            passphrase,
            secret_base32: secret_b32,
        })
    }

    /// Join a room using a passphrase (e.g. "pomme-tigre-bleu-ocean")
    /// or a raw base32 secret.
    pub fn join_room(&self, room_name: &str, input: &str) -> Result<(), String> {
        let clean = input.trim();
        if clean.is_empty() {
            return Err("Code d'accès requis.".into());
        }

        let secret_b32 = if clean.contains('-') {
            passphrase_to_secret(clean)?
        } else {
            clean.replace(' ', "").to_uppercase()
        };

        let passphrase = secret_to_passphrase(&secret_b32);
        let totp = build_totp(&secret_b32, room_name)?;

        let mut state = self.state.lock().unwrap();
        *state = Some(RoomState {
            room_name: room_name.to_string(),
            totp,
            secret_base32: secret_b32.clone(),
            passphrase,
        });

        save_room(room_name, &secret_b32);

        Ok(())
    }

    pub fn leave_room(&self) {
        *self.state.lock().unwrap() = None;
        delete_room_file();
    }

    pub fn current_code(&self) -> Option<String> {
        let state = self.state.lock().unwrap();
        state.as_ref().map(|s| s.totp.generate(now_secs()))
    }

    /// Verify a TOTP code (allows +/- 1 step of clock skew).
    pub fn verify_code(&self, code: &str) -> bool {
        let state = self.state.lock().unwrap();
        match state.as_ref() {
            None => false,
            Some(s) => {
                let time = now_secs();
                for offset in [0i64, -1, 1] {
                    let t = (time as i64 + offset * TOTP_STEP as i64) as u64;
                    if s.totp.generate(t) == code {
                        return true;
                    }
                }
                false
            }
        }
    }

    pub fn is_joined(&self) -> bool {
        self.state.lock().unwrap().is_some()
    }

    pub fn get_status(&self) -> RoomStatus {
        let state = self.state.lock().unwrap();
        match state.as_ref() {
            None => RoomStatus {
                joined: false,
                room_name: String::new(),
                passphrase: String::new(),
                current_code: String::new(),
                seconds_remaining: 0,
            },
            Some(s) => {
                let time = now_secs();
                let code = s.totp.generate(time);
                let remaining = TOTP_STEP - (time % TOTP_STEP);
                RoomStatus {
                    joined: true,
                    room_name: s.room_name.clone(),
                    passphrase: s.passphrase.clone(),
                    current_code: code,
                    seconds_remaining: remaining,
                }
            }
        }
    }

    pub fn get_secret(&self) -> Option<String> {
        self.state.lock().unwrap().as_ref().map(|s| s.secret_base32.clone())
    }

}

pub struct CreateRoomOutput {
    pub passphrase: String,
    pub secret_base32: String,
}

// ── TOTP helpers ───────────────────────────────────────────

fn build_totp(secret_base32: &str, _room_name: &str) -> Result<TOTP, String> {
    let secret = Secret::Encoded(secret_base32.to_string())
        .to_bytes()
        .map_err(|e| format!("Secret invalide : {e}"))?;

    TOTP::new(
        Algorithm::SHA1,
        TOTP_DIGITS,
        1,
        TOTP_STEP,
        secret,
    )
    .map_err(|e| format!("Erreur TOTP : {e}"))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Passphrase <-> secret conversion ───────────────────────

/// Convert a base32 secret to a 4-word passphrase.
/// Takes the first 4 bytes of the decoded secret as word indices.
fn secret_to_passphrase(secret_b32: &str) -> String {
    let bytes = data_encoding::BASE32.decode(secret_b32.as_bytes()).unwrap_or_default();
    let mut words = Vec::with_capacity(4);
    for i in 0..4 {
        let idx = *bytes.get(i).unwrap_or(&0) as usize;
        words.push(WORDLIST[idx % 256]);
    }
    words.join("-")
}

/// Convert a 4-word passphrase back to a base32 secret.
/// We reconstruct the 4 bytes, then pad to 20 bytes using a deterministic
/// expansion (SHA1 of the 4 bytes) to get a proper TOTP secret length.
fn passphrase_to_secret(passphrase: &str) -> Result<String, String> {
    let parts: Vec<&str> = passphrase.split('-').collect();
    if parts.len() != 4 {
        return Err(format!(
            "Le code d'accès doit contenir 4 mots séparés par des tirets (reçu : {}).",
            parts.len()
        ));
    }

    let mut seed = [0u8; 4];
    for (i, word) in parts.iter().enumerate() {
        let lower = word.to_lowercase();
        let idx = WORDLIST
            .iter()
            .position(|w| *w == lower)
            .ok_or_else(|| format!("Mot inconnu : « {} ». Vérifiez l'orthographe.", word))?;
        seed[i] = idx as u8;
    }

    // Expand 4 bytes to 20 bytes deterministically using SHA1
    use sha1::Digest;
    let mut hasher = sha1::Sha1::new();
    hasher.update(&seed);
    // Feed it multiple rounds for length
    let hash1 = hasher.finalize();
    let secret_bytes: Vec<u8> = hash1.iter().copied().take(20).collect();

    Ok(data_encoding::BASE32.encode(&secret_bytes))
}
