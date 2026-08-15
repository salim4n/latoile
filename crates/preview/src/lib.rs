//! Live previews. Supervises each project's dev server (one active preview
//! per project, enforced with the domain), allocates ports, and reverse
//! proxies traffic so the UI embeds the app under construction.
//!
//! A preview always serves the project's declared work branch; when it stops
//! being true (build failure, branch moved), the preview reports `stale` or
//! `error` — it never silently serves the wrong thing. Dev-server processes
//! are supervised and reaped like agent processes.
