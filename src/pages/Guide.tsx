import { useLang } from "../lib/i18n";
import { GuideFr } from "./Guide.fr";
import { GuideEn } from "./Guide.en";

export function Guide() {
  const { lang } = useLang();
  return lang === "en" ? <GuideEn /> : <GuideFr />;
}
