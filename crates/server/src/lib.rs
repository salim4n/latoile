//! The HTTP interface — the only crate that knows axum. Handlers extract,
//! validate, and delegate to `latoile-app` use cases; they contain no logic.
//!
//! Also owns: the SSE event stream (cursor-resume over `EVENT.seq`), the
//! embedded web assets, and token authentication. Every route sits behind the
//! token, the preview proxy included. Error responses are `{code, message}`;
//! internal error chains go to tracing, never to the client.
