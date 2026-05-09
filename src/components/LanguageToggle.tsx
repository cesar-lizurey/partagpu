import { useLang } from "../lib/i18n";

/** Toggle button between French and English. Shows the flag of the
 *  language we'd switch *to* (so the user sees where the click leads). */
export function LanguageToggle() {
  const { lang, setLang } = useLang();
  const next = lang === "fr" ? "en" : "fr";
  const title =
    lang === "fr" ? "Switch to English" : "Passer en français";
  const aria =
    lang === "fr"
      ? "Switch language to English"
      : "Changer la langue en français";

  return (
    <button
      type="button"
      className="lang-toggle"
      onClick={() => setLang(next)}
      title={title}
      aria-label={aria}
    >
      {next === "en" ? <FlagGB /> : <FlagFR />}
    </button>
  );
}

function FlagFR() {
  return (
    <svg
      viewBox="0 0 3 2"
      width="28"
      height="20"
      aria-hidden="true"
      className="lang-toggle__flag"
    >
      <rect width="1" height="2" x="0" fill="#0055A4" />
      <rect width="1" height="2" x="1" fill="#FFFFFF" />
      <rect width="1" height="2" x="2" fill="#EF4135" />
    </svg>
  );
}

function FlagGB() {
  return (
    <svg
      viewBox="0 0 60 30"
      width="28"
      height="20"
      aria-hidden="true"
      className="lang-toggle__flag"
    >
      <clipPath id="lt-uk">
        <path d="M0,0 v30 h60 v-30 z" />
      </clipPath>
      <clipPath id="lt-uk-diag">
        <path d="M30,15 h30 v15 z M30,15 v15 h-30 z M30,15 h-30 v-15 z M30,15 v-15 h30 z" />
      </clipPath>
      <g clipPath="url(#lt-uk)">
        <path d="M0,0 v30 h60 v-30 z" fill="#012169" />
        <path d="M0,0 L60,30 M60,0 L0,30" stroke="#FFFFFF" strokeWidth="6" />
        <path
          d="M0,0 L60,30 M60,0 L0,30"
          stroke="#C8102E"
          strokeWidth="4"
          clipPath="url(#lt-uk-diag)"
        />
        <path
          d="M30,0 v30 M0,15 h60"
          stroke="#FFFFFF"
          strokeWidth="10"
        />
        <path
          d="M30,0 v30 M0,15 h60"
          stroke="#C8102E"
          strokeWidth="6"
        />
      </g>
    </svg>
  );
}
