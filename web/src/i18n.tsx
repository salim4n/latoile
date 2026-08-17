// i18n (D11): FR + EN from day one, hand-rolled dictionaries, FR default.
// The localStorage key « latoile-lang » is the mockups' own — the language
// chosen in the design gallery carries into the app.

import { createContext, useContext, useEffect, useState } from "react";
import type { ReactNode } from "react";

export type Lang = "fr" | "en";

const STORAGE_KEY = "latoile-lang";

const fr = {
  "nav.inbox": "Inbox",
  "nav.projects": "Projets",
  "nav.create": "Créer",
  "nav.settings": "Réglages",
  "nav.aria": "Navigation principale",
  "lang.aria": "Langue d'affichage",
  "shell.connected": "connecté",
  "shell.disconnected": "déconnecté",
  "shell.instance": "instance locale",
  "shell.back.projects": "‹ Projets",
  "shell.back.inbox": "‹ Inbox",

  "token.title": "LaToile",
  "token.lead": "Collez le token affiché au démarrage par `latoile serve`.",
  "token.label": "Token",
  "token.submit": "Entrer",
  "token.error": "Token refusé — vérifiez-le et réessayez.",

  "inbox.approvals": "Approbations en attente",
  "inbox.blocked": "Runs bloqués",
  "inbox.projects": "Projets actifs",
  "inbox.review.badge": "Review à trancher",
  "inbox.review.link": "Examiner la review →",
  "inbox.permission.badge": "Permission requise",
  "inbox.permission.allow": "Autoriser",
  "inbox.permission.deny": "Refuser",
  "inbox.empty.title": "Aucune décision en attente",
  "inbox.empty.body": "Tout roule — le Manager vous préviendra dès qu'une approbation ou une permission aura besoin de vous.",
  "inbox.empty.cta": "Voir les projets",
  "inbox.error.title": "Impossible de joindre LaToile",
  "inbox.error.body": "Le serveur local ne répond pas — vérifiez que le tunnel (cloudflared) est actif, puis réessayez.",
  "inbox.error.retry": "Réessayer",

  "status.draft": "Brouillon",
  "status.specced": "Spec prête",
  "status.building": "En cours",
  "status.live": "Live",

  "projects.title": "Projets",
  "projects.new": "Nouveau projet",
  "projects.empty.title": "Aucun projet",
  "projects.empty.body": "Créez votre premier projet : choisissez un dépôt, écrivez un brief, le Manager s'occupe du reste.",

  "new.title": "Nouveau projet",
  "new.repo.legend": "Dépôt GitHub",
  "new.repo.error": "Impossible de lister vos dépôts — vérifiez le token GitHub dans la vault.",
  "new.brief.label": "Brief initial",
  "new.brief.hint": "Le Manager découpera ce brief en tâches et proposera un plan. 3 à 5 phrases suffisent.",
  "new.submit": "Créer le projet",
  "new.sending": "Création en cours…",
  "new.sending.note": "Le Manager prépare le dépôt et la première planification.",
  "new.failed": "La création a échoué — réessayez.",

  "tabs.chat": "Chat",
  "tabs.board": "Board",
  "tabs.preview": "Preview",
  "chat.you": "Vous",
  "chat.manager": "Manager",
  "chat.placeholder": "Message au Manager…",
  "chat.send": "Envoyer",
  "chat.empty": "Décrivez ce que vous voulez construire — le Manager découpe, planifie et lance.",
  "board.ready": "Prêt",
  "board.progress": "En cours",
  "board.review": "Review",
  "board.done": "Terminé",
  "board.empty": "Aucune tâche pour l'instant.",
  "preview.live": "live",
  "preview.url": "preview locale",
  "preview.start": "Démarrer la preview",
  "preview.off.title": "Aucune preview",
  "preview.off.body": "Le dev server du projet ne tourne pas. Démarrez-le pour voir l'app en construction.",
  "preview.error.badge": "build échoué",
  "preview.error.title": "Le build a échoué",
  "preview.error.body": "Le Frontend a été notifié et prépare une correction.",
  "preview.retry": "Relancer",
  "preview.mobile": "Mobile",
  "preview.desktop": "Desktop",

  "review.badge.pending": "Review à trancher",
  "review.badge.approved": "Approuvé",
  "review.badge.rejected": "Changements demandés",
  "review.title": "Verdict du Reviewer",
  "review.no.details": "Le verdict détaillé (diff, findings, comparaison maquette / rendu) arrive avec la passe orchestrateur — la demande elle-même est ci-dessous.",
  "review.approve": "Approuver",
  "review.changes": "Demander des changements",
  "review.decided.approved": "Review approuvée par vous.",
  "review.decided.rejected": "Changements demandés — le Manager a été notifié.",
  "review.gone": "Cette approbation a déjà été tranchée.",
  "review.back": "Retour à l'Inbox",

  "auth.title": "Agent Claude",
  "auth.body": "LaToile pilote votre abonnement Claude Code. Connectez-le une fois : tout le reste suit.",
  "auth.connect": "Connecter Claude",
  "auth.connect.codex": "Connecter Codex",
  "auth.enter.code": "Entrez ce code sur la page :",
  "auth.waiting": "En attente de confirmation…",
  "auth.open": "Ouvrir la page de connexion ↗",
  "auth.paste": "Collez le code ici",
  "auth.validate": "Valider",
  "auth.connected": "Claude connecté",
  "auth.failed": "La connexion a échoué",
  "auth.expired": "Le délai a expiré",
  "auth.retry": "Réessayer",
  "settings.title": "Réglages",
  "settings.connections": "Connexions",
  "settings.team": "Équipe IA",
  "settings.team.note": "Un changement de provider s'applique aux nouvelles sessions.",
  "settings.save": "Enregistrer",
  "settings.saved": "Enregistré ✓",
  "settings.save.error": "L'enregistrement a échoué — réessayez.",
  "cost.claude": "Abonnement Claude Code",
  "cost.codex": "Abonnement ChatGPT / Codex",
  "auth.connected.detail": "Connecté",
  "auth.disconnect": "Déconnecter",
  "auth.disconnect.confirm": "Confirmer la déconnexion ?",
  "auth.cancel": "Annuler",
  "auth.action.error": "L'action a échoué — vérifiez que le serveur et le CLI sont disponibles, puis réessayez.",
  "inbox.auth.banner": "Aucun agent connecté — connectez-en un pour démarrer.",
  "inbox.auth.cta": "Ouvrir les réglages →",
  "state.loading": "Chargement…",
} as const;

export type Key = keyof typeof fr;

const en: Record<Key, string> = {
  "nav.inbox": "Inbox",
  "nav.projects": "Projects",
  "nav.create": "Create",
  "nav.settings": "Settings",
  "nav.aria": "Main navigation",
  "lang.aria": "Display language",
  "shell.connected": "connected",
  "shell.disconnected": "disconnected",
  "shell.instance": "local instance",
  "shell.back.projects": "‹ Projects",
  "shell.back.inbox": "‹ Inbox",

  "token.title": "LaToile",
  "token.lead": "Paste the token printed at startup by `latoile serve`.",
  "token.label": "Token",
  "token.submit": "Sign in",
  "token.error": "Token refused — check it and try again.",

  "inbox.approvals": "Pending approvals",
  "inbox.blocked": "Blocked runs",
  "inbox.projects": "Active projects",
  "inbox.review.badge": "Review to decide",
  "inbox.review.link": "Open review →",
  "inbox.permission.badge": "Permission required",
  "inbox.permission.allow": "Allow",
  "inbox.permission.deny": "Deny",
  "inbox.empty.title": "No pending decisions",
  "inbox.empty.body": "All clear — the Manager will ping you as soon as an approval or a permission needs you.",
  "inbox.empty.cta": "View projects",
  "inbox.error.title": "Can't reach LaToile",
  "inbox.error.body": "The local server is not responding — check that the (cloudflared) tunnel is up, then try again.",
  "inbox.error.retry": "Retry",

  "status.draft": "Draft",
  "status.specced": "Spec ready",
  "status.building": "In progress",
  "status.live": "Live",

  "projects.title": "Projects",
  "projects.new": "New project",
  "projects.empty.title": "No projects",
  "projects.empty.body": "Create your first project: pick a repository, write a brief, the Manager handles the rest.",

  "new.title": "New project",
  "new.repo.legend": "GitHub repository",
  "new.repo.error": "Could not list your repositories — check the GitHub token in the vault.",
  "new.brief.label": "Initial brief",
  "new.brief.hint": "The Manager will split this brief into tasks and propose a plan. 3 to 5 sentences is enough.",
  "new.submit": "Create the project",
  "new.sending": "Creating…",
  "new.sending.note": "The Manager is preparing the repository and the first plan.",
  "new.failed": "Creation failed — try again.",

  "tabs.chat": "Chat",
  "tabs.board": "Board",
  "tabs.preview": "Preview",
  "chat.you": "You",
  "chat.manager": "Manager",
  "chat.placeholder": "Message the Manager…",
  "chat.send": "Send",
  "chat.empty": "Describe what you want to build — the Manager splits, plans and dispatches.",
  "board.ready": "Ready",
  "board.progress": "In progress",
  "board.review": "Review",
  "board.done": "Done",
  "board.empty": "No tasks yet.",
  "preview.live": "live",
  "preview.url": "local preview",
  "preview.start": "Start the preview",
  "preview.off.title": "No preview",
  "preview.off.body": "The project's dev server is not running. Start it to see the app being built.",
  "preview.error.badge": "build failed",
  "preview.error.title": "Build failed",
  "preview.error.body": "Frontend has been notified and is working on a fix.",
  "preview.retry": "Restart",
  "preview.mobile": "Mobile",
  "preview.desktop": "Desktop",

  "review.badge.pending": "Review to decide",
  "review.badge.approved": "Approved",
  "review.badge.rejected": "Changes requested",
  "review.title": "Reviewer verdict",
  "review.no.details": "The detailed verdict (diff, findings, mockup vs render comparison) lands with the orchestrator pass — the request itself is below.",
  "review.approve": "Approve",
  "review.changes": "Request changes",
  "review.decided.approved": "Review approved by you.",
  "review.decided.rejected": "Changes requested — the Manager has been notified.",
  "review.gone": "This approval has already been decided.",
  "review.back": "Back to Inbox",

  "auth.title": "Claude agent",
  "auth.body": "LaToile drives your Claude Code subscription. Connect it once — everything else follows.",
  "auth.connect": "Connect Claude",
  "auth.connect.codex": "Connect Codex",
  "auth.enter.code": "Enter this code on the page:",
  "auth.waiting": "Waiting for confirmation…",
  "auth.open": "Open the login page ↗",
  "auth.paste": "Paste the code here",
  "auth.validate": "Validate",
  "auth.connected": "Claude connected",
  "auth.failed": "Connection failed",
  "auth.expired": "The challenge expired",
  "auth.retry": "Retry",
  "settings.title": "Settings",
  "settings.connections": "Connections",
  "settings.team": "AI team",
  "settings.team.note": "A provider change applies to new sessions.",
  "settings.save": "Save",
  "settings.saved": "Saved ✓",
  "settings.save.error": "Saving failed — try again.",
  "cost.claude": "Claude Code subscription",
  "cost.codex": "ChatGPT / Codex subscription",
  "auth.connected.detail": "Connected",
  "auth.disconnect": "Disconnect",
  "auth.disconnect.confirm": "Confirm disconnect?",
  "auth.cancel": "Cancel",
  "auth.action.error": "The action failed — check that the server and CLI are available, then try again.",
  "inbox.auth.banner": "No agent connected — connect one to get started.",
  "inbox.auth.cta": "Open settings →",
  "state.loading": "Loading…",
};

const dictionaries: Record<Lang, Record<Key, string>> = { fr, en };

interface LangState {
  lang: Lang;
  t: (key: Key) => string;
  setLang: (lang: Lang) => void;
}

const LangContext = createContext<LangState>({
  lang: "fr",
  t: (key) => fr[key],
  setLang: () => {},
});

function initialLang(): Lang {
  try {
    return localStorage.getItem(STORAGE_KEY) === "en" ? "en" : "fr";
  } catch {
    return "fr";
  }
}

export function LangProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(initialLang);

  useEffect(() => {
    document.documentElement.lang = lang;
    try {
      localStorage.setItem(STORAGE_KEY, lang);
    } catch {
      // private mode: the language just won't persist
    }
  }, [lang]);

  const value: LangState = {
    lang,
    t: (key) => dictionaries[lang][key] ?? fr[key] ?? key,
    setLang: setLangState,
  };
  return <LangContext.Provider value={value}>{children}</LangContext.Provider>;
}

export function useT() {
  return useContext(LangContext);
}

export function LangToggle() {
  const { lang, setLang, t } = useT();
  return (
    <div className="lang-toggle" role="group" aria-label={t("lang.aria")}>
      {(["fr", "en"] as Lang[]).map((l) => (
        <button
          key={l}
          type="button"
          aria-pressed={lang === l}
          onClick={() => setLang(l)}
        >
          {l.toUpperCase()}
        </button>
      ))}
    </div>
  );
}
