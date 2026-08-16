// The state blocks every screen shares, straight from the mockups:
// EmptyState (dashed card, headline + why + action), ErrorState (danger
// variant + retry), and skeletons shaped like the content they replace.

import type { ReactNode } from "react";
import { useT } from "../i18n";
import { WarningIcon, CheckIcon } from "./icons";

export function EmptyState({
  title,
  body,
  action,
}: {
  title: string;
  body: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      <CheckIcon />
      <h2>{title}</h2>
      <p>{body}</p>
      {action}
    </div>
  );
}

export function ErrorState({
  title,
  body,
  onRetry,
}: {
  title: string;
  body: string;
  onRetry: () => void;
}) {
  const { t } = useT();
  return (
    <div className="empty empty--error">
      <WarningIcon />
      <h2>{title}</h2>
      <p>{body}</p>
      <button className="btn btn--primary" type="button" onClick={onRetry}>
        {t("inbox.error.retry")}
      </button>
    </div>
  );
}

/// Two approval-ish cards, one command card, three rows — the Inbox shape.
export function Skeletons() {
  return (
    <div aria-busy="true">
      <div className="sec">
        <h2 className="sec-title">
          <span className="skel" style={{ display: "inline-block", width: 190, height: 16, verticalAlign: "middle" }} />
        </h2>
        {[72, 88].map((w) => (
          <div className="card item" aria-hidden="true" key={w}>
            <div className="skel skel-badge" />
            <div className="skel skel-title" style={{ width: `${w}%` }} />
            <div className="skel skel-line" />
          </div>
        ))}
      </div>
      <div className="sec">
        <h2 className="sec-title">
          <span className="skel" style={{ display: "inline-block", width: 120, height: 16, verticalAlign: "middle" }} />
        </h2>
        <div className="card item" aria-hidden="true">
          <div className="skel skel-badge" style={{ width: 132 }} />
          <div className="skel skel-title" style={{ width: "46%" }} />
          <div className="skel skel-cmd" />
        </div>
      </div>
      <div className="sec">
        <h2 className="sec-title">
          <span className="skel" style={{ display: "inline-block", width: 110, height: 16, verticalAlign: "middle" }} />
        </h2>
        {[38, 64, 30].map((w) => (
          <div className="card item" aria-hidden="true" key={w}>
            <div className="skel skel-title" style={{ marginTop: 0, width: `${w}%` }} />
            <div className="skel skel-line skel-line--short" />
          </div>
        ))}
      </div>
    </div>
  );
}
