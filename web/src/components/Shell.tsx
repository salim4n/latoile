// AppShell — topbar + mobile tab bar ↔ desktop sidebar (same destinations,
// same order: Inbox, Projets, Créer). The connection dot mirrors the SSE
// channel's state (green up, danger down), per the mockups' note.

import type { ReactNode } from "react";
import { Link, useLocation } from "react-router-dom";
import { useEffect, useState } from "react";
import { LangToggle, useT } from "../i18n";
import { onStatus, type ConnStatus } from "../events";
import { InboxIcon, PlusIcon, ProjectsIcon } from "./icons";

function Conn() {
  const { t } = useT();
  const [status, setStatus] = useState<ConnStatus>("up");
  useEffect(() => onStatus(setStatus), []);
  const up = status === "up";
  return (
    <span className={up ? "conn" : "conn conn--down"}>
      <span className="dot" aria-hidden="true" />
      {t(up ? "shell.connected" : "shell.disconnected")}
    </span>
  );
}

function NavLinks({ current }: { current: string }) {
  const { t } = useT();
  const items = [
    { to: "/", key: "inbox", label: t("nav.inbox"), icon: <InboxIcon /> },
    { to: "/projects", key: "projects", label: t("nav.projects"), icon: <ProjectsIcon /> },
    { to: "/projects/new", key: "create", label: t("nav.create"), icon: <PlusIcon /> },
  ];
  return (
    <>
      {items.map((item) => (
        <Link
          key={item.key}
          to={item.to}
          aria-current={current === item.key ? "page" : undefined}
        >
          {item.icon}
          <span>{item.label}</span>
        </Link>
      ))}
    </>
  );
}

export function Shell({
  back,
  title,
  crumb,
  wide,
  children,
}: {
  /// Mobile topbar back link label; omitted on the Inbox.
  back?: string;
  /// Mobile topbar title.
  title: string;
  /// Desktop crumb (e.g. "Projets / LaToile").
  crumb?: ReactNode;
  /// Wider measure for the project workspace (board + preview).
  wide?: boolean;
  children: ReactNode;
}) {
  const { t } = useT();
  const location = useLocation();
  // The current tab follows the URL: Inbox (and reviews), then Projects,
  // then Create — same destinations, same order, both bars.
  const path = location.pathname;
  const current =
    path === "/projects/new" ? "create" : path.startsWith("/projects") ? "projects" : "inbox";

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="side-logo">LaToile</div>
        <nav className="side-nav" aria-label={t("nav.aria")}>
          <NavLinks current={current} />
        </nav>
        <div className="side-foot">
          <LangToggle />
          <div className="side-user">
            <span className="avatar" aria-hidden="true">S</span>
            <div>
              <div className="who">salim4n</div>
              <div className="what">{t("shell.instance")}</div>
            </div>
          </div>
        </div>
      </aside>

      <div className="body-col">
        <header className="topbar">
          {back ? (
            <Link className="back" to={current === "projects" ? "/projects" : "/"}>
              {back}
            </Link>
          ) : (
            <span className="wordmark">LaToile</span>
          )}
          <span className="title">{title}</span>
          <span className="crumb">{crumb ?? title}</span>
          <div className="topbar-right">
            <Conn />
            <LangToggle />
          </div>
        </header>

        <main>
          <div className={wide ? "main-inner main-inner--wide" : "main-inner"}>
            {children}
          </div>
        </main>
      </div>

      <nav className="tabbar" aria-label={t("nav.aria")}>
        <NavLinks current={current} />
      </nav>
    </div>
  );
}
