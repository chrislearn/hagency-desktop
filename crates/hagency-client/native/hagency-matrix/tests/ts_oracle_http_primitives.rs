//! TS test oracle — `tests/bridge-matrix-http-primitives.test.js`,
//! describe block `an omitted base URL is a throw, not a silent fallback`
//! (`:174-215`).
//!
//! TS rule: the Matrix base URL is a REQUIRED input to every primitive. An
//! omitted, empty or whitespace URL is a refusal, never a silent default — a
//! default would send a caller's credential to this deployment's own server.
//!
//! The native counterpart is the host endpoint validator
//! (`hagency-matrix/src/config.rs:245` `host_endpoint`), reached publicly
//! through `RequestPacing::new`. Each test asserts the TS-visible outcome;
//! where native deliberately differs the test is `#[ignore]`d and listed as a
//! parity gap in `.peer/report-66.md` (no product code is changed here).
use hagency_matrix::{Error, RequestPacing};
use std::time::Duration;

fn interval() -> Duration {
    Duration::from_millis(100)
}

fn accepts(endpoint: &str) -> bool {
    RequestPacing::new(endpoint, interval()).is_ok()
}

/// TS: `an empty or whitespace URL is refused too, not concatenated` —
/// `for (const bad of ['', '   ', null, 0])` each rejects with
/// `/requires a Matrix base URL/`.
///
/// Native refuses an empty or whitespace endpoint as `Error::Config` (the
/// Rust equivalent of the same refusal; the non-string `null`/`0` cases have
/// no Rust value).
#[test]
fn ts_empty_or_whitespace_base_url_is_refused() {
    for bad in ["", "   ", "\t", " \n "] {
        assert!(
            !accepts(bad),
            "base URL {bad:?} must be refused, not concatenated"
        );
        assert_eq!(
            RequestPacing::new(bad, interval()).map(|_| ()),
            Err(Error::Config)
        );
    }
}

/// TS: the canonical origin WITH a single trailing slash is accepted — this is
/// how the TS test's `baseUrl` is written (`http://127.0.0.1:${port}`), and
/// the native accepted form (`url.path() == "/"`).
#[test]
fn ts_origin_with_a_single_trailing_slash_is_accepted() {
    assert!(accepts("http://127.0.0.1:13443/"));
    assert!(accepts("https://us.example/"));
}

/// TS: `a trailing slash is trimmed rather than doubled` — the primitive
/// accepts `${baseUrl}///` and reaches `/_matrix/client/v3/account/whoami`
/// with a single slash, because "some homeservers 404 `//_matrix/...`".
///
/// Native refuses more than one trailing slash: `host_endpoint` requires
/// `url.path() == "/"` and `url.as_str() == endpoint`.
#[test]
#[ignore = "parity gap: TS trims any number of trailing slashes (bridge-matrix-http-primitives.test.js:208-214); native host_endpoint refuses a path other than \"/\""]
fn ts_trailing_slash_is_trimmed_rather_than_doubled() {
    assert!(
        accepts("http://127.0.0.1:13443///"),
        "TS accepts a base URL with three trailing slashes and normalizes it to a single slash"
    );
}

/// TS: an origin with NO trailing slash is also accepted — the TS test's
/// `baseUrl` is assigned without one (`:62`) and is passed to every primitive
/// unchanged (`:118-123`).
///
/// Native refuses the slash-less form: `Url::parse` normalizes `as_str()` to
/// add `/`, so `url.as_str() != endpoint` fails the canonical-form check.
#[test]
#[ignore = "parity gap: TS accepts a slash-less origin (bridge-matrix-http-primitives.test.js:62,118-123); native host_endpoint requires the canonical \"http://host:port/\" form"]
fn ts_origin_without_a_trailing_slash_is_accepted() {
    assert!(
        accepts("http://127.0.0.1:13443"),
        "TS accepts a base URL without a trailing slash"
    );
}

/// TS: the base URL is REQUIRED — a value that is not a usable origin is a
/// refusal, not a silent fallback to this deployment's own server. Native
/// refuses a URL that is not a bare `http`/`https` origin (credentials, a
/// path, a query, a fragment, a non-http scheme).
#[test]
fn ts_url_must_be_a_bare_http_origin() {
    for bad in [
        "http://user:pass@127.0.0.1:13443/",
        "http://127.0.0.1:13443/_matrix",
        "http://127.0.0.1:13443/?x=1",
        "http://127.0.0.1:13443/#f",
        "ftp://127.0.0.1:13443/",
        "http://",
    ] {
        assert!(!accepts(bad), "base URL {bad:?} is not a bare origin");
    }
}
