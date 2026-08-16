// i18n (D11): FR by default, the toggle switches, and the mockups' storage
// key is honored.

import { render, screen, fireEvent } from "@testing-library/react";
import { LangProvider, LangToggle, useT } from "./i18n";

function Probe() {
  const { t } = useT();
  return <p>{t("nav.projects")}</p>;
}

describe("i18n", () => {
  beforeEach(() => localStorage.clear());

  it("defaults to French", () => {
    render(
      <LangProvider>
        <Probe />
      </LangProvider>,
    );
    expect(screen.getByText("Projets")).toBeTruthy();
  });

  it("switches to English and persists the mockups' key", () => {
    render(
      <LangProvider>
        <LangToggle />
        <Probe />
      </LangProvider>,
    );
    fireEvent.click(screen.getByText("EN"));
    expect(screen.getByText("Projects")).toBeTruthy();
    expect(localStorage.getItem("latoile-lang")).toBe("en");
    expect(document.documentElement.lang).toBe("en");
  });

  it("reads a stored language", () => {
    localStorage.setItem("latoile-lang", "en");
    render(
      <LangProvider>
        <Probe />
      </LangProvider>,
    );
    expect(screen.getByText("Projects")).toBeTruthy();
  });
});
