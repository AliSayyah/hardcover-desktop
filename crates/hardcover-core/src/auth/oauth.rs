//! Token endpoint calls: code exchange, refresh, revoke, discovery.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use super::{OAuthConfig, OAuthEndpoints, Tokens};
use crate::http::{HttpRequest, HttpResponse, HttpTransport, Method};
use crate::{Error, Result};

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<Value>,
    scope: Option<String>,
}

/// OAuth error codes (RFC 6749 §5.2) that say the grant itself is dead: the
/// code or refresh token was used, revoked or expired, or the client is
/// unknown. Retrying can't help, and only these end a session.
const DEAD_GRANT: [&str; 3] = ["invalid_grant", "invalid_client", "unauthorized_client"];

/// Why a token request failed.
#[derive(Debug)]
pub(crate) enum TokenError {
    /// The token endpoint answered with an OAuth error. `code` is its
    /// `error`; `message` adds the `error_description`.
    OAuth { code: String, message: String },
    /// No OAuth answer: network trouble, a 5xx or 429, an unreadable success,
    /// or a 4xx that isn't an OAuth error (a captive portal's or a proxy's
    /// page). The grant may or may not have been used.
    Transient(Error),
}

impl TokenError {
    /// True if the grant is dead for good ([`DEAD_GRANT`]). Any other
    /// failure leaves the session as it was.
    pub(crate) fn grant_is_dead(&self) -> bool {
        matches!(self, Self::OAuth { code, .. } if DEAD_GRANT.contains(&code.as_str()))
    }

    /// The failure as a core error: an OAuth answer is `Auth`.
    pub(crate) fn into_error(self) -> Error {
        match self {
            Self::OAuth { message, .. } => Error::Auth(message),
            Self::Transient(e) => e,
        }
    }
}

/// The returned tokens always carry a scope: a response without one granted
/// exactly what was requested (RFC 6749 §5.1), so that is recorded instead.
pub(crate) fn exchange_code(
    http: &dyn HttpTransport,
    cfg: &OAuthConfig,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> std::result::Result<Tokens, TokenError> {
    let tokens = token_request(
        http,
        cfg,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
            ("client_id", cfg.client_id()),
        ],
    )?;
    Ok(match tokens.scope() {
        Some(_) => tokens,
        None => tokens.with_scope(Some(cfg.scopes().join(" "))),
    })
}

/// One attempt only: refresh tokens rotate, and replaying one revokes the chain.
/// The result's scope is `None` when the response left it out, which means it
/// did not change (RFC 6749 §5.1); the caller keeps the previous one.
pub(crate) fn refresh(
    http: &dyn HttpTransport,
    cfg: &OAuthConfig,
    refresh_token: &str,
) -> std::result::Result<Tokens, TokenError> {
    token_request(
        http,
        cfg,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", cfg.client_id()),
        ],
    )
}

fn token_request(
    http: &dyn HttpTransport,
    cfg: &OAuthConfig,
    form: &[(&str, &str)],
) -> std::result::Result<Tokens, TokenError> {
    let req = HttpRequest::new(Method::Post, &cfg.endpoints().token)
        .header("accept", "application/json")
        .form(form)
        .timeout(TIMEOUT);
    let resp = http.send(req).map_err(TokenError::Transient)?;
    parse_token_response(&resp, chrono::Utc::now())
}

pub(crate) fn parse_token_response(
    resp: &HttpResponse,
    now: chrono::DateTime<chrono::Utc>,
) -> std::result::Result<Tokens, TokenError> {
    if (200..300).contains(&resp.status) {
        let t: TokenResponse = serde_json::from_slice(&resp.body)
            .map_err(|e| TokenError::Transient(Error::Decode(format!("token response: {e}"))))?;
        let expires_in = t.expires_in.as_ref().and_then(crate::api::wire_i64);
        let expires_at = expires_in.map(|s| now + chrono::Duration::seconds(s));
        // A blank scope says nothing about the grant; treat it as missing
        // rather than as a grant of nothing.
        let scope = t.scope.filter(|s| !s.trim().is_empty());
        return Ok(Tokens::new(t.access_token, t.refresh_token, expires_at).with_scope(scope));
    }
    let body: Value = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);
    let field = |k: &str| {
        body.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    // Only a 4xx carrying an OAuth `error` is the server's verdict on the
    // grant. A 5xx or 429 is the server's trouble, and a 4xx without one
    // (an HTML page from a captive portal, a WAF or a CDN) never reached it.
    match field("error") {
        Some(code) if (400..500).contains(&resp.status) && resp.status != 429 => {
            let message = match field("error_description") {
                Some(d) => format!("{code}: {d}"),
                None => code.clone(),
            };
            Err(TokenError::OAuth { code, message })
        }
        _ => Err(TokenError::Transient(Error::Network(format!(
            "the token endpoint answered HTTP {}",
            resp.status
        )))),
    }
}

/// RFC 7009 revocation. Revoking either token revokes both.
pub(crate) fn revoke(http: &dyn HttpTransport, cfg: &OAuthConfig, tokens: &Tokens) -> Result<()> {
    let (token, hint) = match tokens.refresh_token() {
        Some(rt) => (rt, "refresh_token"),
        None => (tokens.access_token(), "access_token"),
    };
    let req = HttpRequest::new(Method::Post, &cfg.endpoints().revoke)
        .form(&[
            ("token", token),
            ("token_type_hint", hint),
            ("client_id", cfg.client_id()),
        ])
        .timeout(TIMEOUT);
    let resp = http.send(req)?;
    if (200..300).contains(&resp.status) {
        Ok(())
    } else {
        Err(Error::Auth(format!(
            "revoking the token failed with HTTP {}",
            resp.status
        )))
    }
}

pub(crate) fn discover(http: &dyn HttpTransport, expected_issuer: &str) -> Result<OAuthEndpoints> {
    let url = format!("{expected_issuer}/.well-known/oauth-authorization-server");
    let resp = http.send(HttpRequest::new(Method::Get, url).timeout(TIMEOUT))?;
    if resp.status != 200 {
        return Err(Error::Auth(format!(
            "discovery failed with HTTP {}",
            resp.status
        )));
    }
    let v: Value = serde_json::from_slice(&resp.body).map_err(|e| Error::Decode(e.to_string()))?;
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Error::Decode(format!("discovery document has no `{k}`")))
    };
    let issuer = s("issuer")?;
    if issuer != expected_issuer {
        return Err(Error::Auth(format!(
            "discovery returned unexpected issuer {issuer}"
        )));
    }
    Ok(OAuthEndpoints {
        issuer,
        authorize: s("authorization_endpoint")?,
        token: s("token_endpoint")?,
        revoke: s("revocation_endpoint")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::fake::FakeTransport;

    fn resp(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            headers: vec![],
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn parses_success() {
        let now = chrono::Utc::now();
        let t = parse_token_response(
            &resp(200, r#"{"access_token":"hc_at_x","refresh_token":"hc_rt_y","expires_in":604800,"token_type":"Bearer","scope":"read:me"}"#),
            now,
        )
        .ok()
        .unwrap();
        assert_eq!(t.access_token(), "hc_at_x");
        assert_eq!(t.refresh_token(), Some("hc_rt_y"));
        assert_eq!(
            t.expires_at(),
            Some(now + chrono::Duration::seconds(604_800))
        );
        assert_eq!(t.scope(), Some("read:me"));
    }

    fn failure(status: u16, body: &str) -> TokenError {
        parse_token_response(&resp(status, body), chrono::Utc::now())
            .err()
            .unwrap()
    }

    #[test]
    fn classifies_failures() {
        let used = failure(
            400,
            r#"{"error":"invalid_grant","error_description":"used"}"#,
        );
        assert!(used.grant_is_dead());
        assert!(
            matches!(&used, TokenError::OAuth { message, .. } if message == "invalid_grant: used")
        );
        assert!(failure(401, r#"{"error":"invalid_client"}"#).grant_is_dead());
        assert!(failure(400, r#"{"error":"unauthorized_client"}"#).grant_is_dead());
        assert!(matches!(failure(503, "oops"), TokenError::Transient(_)));
        assert!(matches!(
            parse_token_response(&resp(200, "not json"), chrono::Utc::now()),
            Err(TokenError::Transient(_))
        ));
    }

    #[test]
    fn only_a_dead_grant_counts_as_rejected() {
        // A captive portal, a WAF or a CDN: no OAuth answer at all.
        let portal = failure(403, "<html><body>Log in to the Wi-Fi</body></html>");
        assert!(matches!(&portal, TokenError::Transient(Error::Network(_))));
        assert!(!portal.grant_is_dead());
        assert!(!failure(400, "").grant_is_dead());
        assert!(!failure(404, r#"{"message":"not found"}"#).grant_is_dead());
        // An OAuth answer that isn't about the grant.
        let other = failure(400, r#"{"error":"invalid_request"}"#);
        assert!(matches!(&other, TokenError::OAuth { code, .. } if code == "invalid_request"));
        assert!(!other.grant_is_dead());
        // The server's own trouble, even with an OAuth body.
        assert!(!failure(429, r#"{"error":"invalid_grant"}"#).grant_is_dead());
        assert!(!failure(500, r#"{"error":"invalid_grant"}"#).grant_is_dead());
    }

    #[test]
    fn code_exchange_records_the_requested_scopes_when_the_response_has_none() {
        let http = FakeTransport::new();
        let cfg = OAuthConfig::new("cid").with_scopes(["read:me", "read:goals"]);
        http.push(
            200,
            r#"{"access_token":"at","refresh_token":"rt","expires_in":60}"#,
        );
        let t = exchange_code(&http, &cfg, "code", "http://127.0.0.1/callback", "v")
            .ok()
            .unwrap();
        assert_eq!(t.scope(), Some("read:me read:goals"));
        // A narrower grant is kept as the server reported it.
        http.push(200, r#"{"access_token":"at","scope":"read:me"}"#);
        let t = exchange_code(&http, &cfg, "code", "http://127.0.0.1/callback", "v")
            .ok()
            .unwrap();
        assert_eq!(t.scope(), Some("read:me"));
    }

    #[test]
    fn refresh_leaves_a_missing_scope_missing() {
        let http = FakeTransport::new();
        http.push(200, r#"{"access_token":"at","refresh_token":"rt"}"#);
        let t = refresh(&http, &OAuthConfig::new("cid"), "rt0")
            .ok()
            .unwrap();
        assert_eq!(t.scope(), None);
    }

    #[test]
    fn a_blank_scope_counts_as_missing() {
        let http = FakeTransport::new();
        http.push(
            200,
            r#"{"access_token":"at","refresh_token":"rt","scope":" "}"#,
        );
        let t = refresh(&http, &OAuthConfig::new("cid"), "rt0")
            .ok()
            .unwrap();
        assert_eq!(t.scope(), None);
    }

    #[test]
    fn revoke_prefers_refresh_token() {
        let http = FakeTransport::new();
        http.push(200, "");
        let cfg = OAuthConfig::new("cid");
        revoke(&http, &cfg, &Tokens::new("at", Some("rt".into()), None)).unwrap();
        let r = &http.requests()[0];
        assert_eq!(r.url, "https://api.hardcover.app/oauth2/revoke");
        assert_eq!(r.form_value("token"), Some("rt"));
        assert_eq!(r.form_value("token_type_hint"), Some("refresh_token"));
        assert_eq!(r.form_value("client_id"), Some("cid"));
    }

    #[test]
    fn discovery_checks_issuer() {
        let http = FakeTransport::new();
        http.push(200, r#"{"issuer":"https://api.hardcover.app","authorization_endpoint":"https://hardcover.app/oauth2/authorize","token_endpoint":"https://api.hardcover.app/oauth2/token","revocation_endpoint":"https://api.hardcover.app/oauth2/revoke"}"#);
        assert_eq!(
            discover(&http, "https://api.hardcover.app").unwrap(),
            OAuthEndpoints::default()
        );
        http.push(200, r#"{"issuer":"https://evil.example","authorization_endpoint":"a","token_endpoint":"b","revocation_endpoint":"c"}"#);
        assert!(matches!(
            discover(&http, "https://api.hardcover.app"),
            Err(Error::Auth(_))
        ));
    }
}
