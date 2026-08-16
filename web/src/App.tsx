// Routes and the token gate: no token → the onboarding screen, and any 401
// anywhere drops back to it (the API layer calls onUnauthorized).

import { useEffect, useState } from "react";
import { BrowserRouter, Route, Routes } from "react-router-dom";
import { getToken, onUnauthorized } from "./api";
import { LangProvider } from "./i18n";
import { TokenScreen } from "./screens/TokenScreen";
import { InboxScreen } from "./screens/InboxScreen";
import { ProjectsScreen } from "./screens/ProjectsScreen";
import { NewProjectScreen } from "./screens/NewProjectScreen";
import { ProjectScreen } from "./screens/ProjectScreen";
import { ReviewScreen } from "./screens/ReviewScreen";

export function App() {
  const [authed, setAuthed] = useState(() => getToken() !== null);
  useEffect(() => onUnauthorized(() => setAuthed(false)), []);

  if (!authed) {
    return (
      <LangProvider>
        <TokenScreen onAccepted={() => setAuthed(true)} />
      </LangProvider>
    );
  }

  return (
    <LangProvider>
      <BrowserRouter>
        <Routes>
          <Route path="/" element={<InboxScreen />} />
          <Route path="/projects" element={<ProjectsScreen />} />
          <Route path="/projects/new" element={<NewProjectScreen />} />
          <Route path="/projects/:id" element={<ProjectScreen />} />
          <Route path="/reviews/:approvalId" element={<ReviewScreen />} />
          <Route path="*" element={<InboxScreen />} />
        </Routes>
      </BrowserRouter>
    </LangProvider>
  );
}
