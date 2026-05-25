use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, bail};
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const TOKEN_ENV: &str = "SYNAPSE_BEARER_TOKEN";
const APPDATA_ENV: &str = "APPDATA";

#[derive(Clone, Debug)]
pub(super) struct HttpAuth {
    token_digest: [u8; 32],
    source: TokenSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TokenSource {
    File(PathBuf),
    Env,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum AuthFailure {
    Missing,
    Malformed,
    Invalid,
}

impl HttpAuth {
    pub(super) fn load() -> anyhow::Result<Self> {
        let (token, source) = load_token()?;
        Ok(Self {
            token_digest: digest_token(&token),
            source,
        })
    }

    #[cfg(test)]
    fn from_token(token: &str) -> Self {
        Self {
            token_digest: digest_token(token),
            source: TokenSource::Env,
        }
    }

    pub(super) const fn source_label(&self) -> &'static str {
        match self.source {
            TokenSource::File(_) => "file",
            TokenSource::Env => "env",
        }
    }

    fn authorize(&self, headers: &HeaderMap) -> Result<(), AuthFailure> {
        let token = bearer_token(headers)?;
        if self.token_matches(token) {
            Ok(())
        } else {
            Err(AuthFailure::Invalid)
        }
    }

    fn token_matches(&self, candidate: &str) -> bool {
        let candidate_digest = digest_token(candidate);
        bool::from(
            self.token_digest
                .as_slice()
                .ct_eq(candidate_digest.as_slice()),
        )
    }
}

pub(super) async fn require_bearer(
    State(auth): State<Arc<HttpAuth>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    match auth.authorize(request.headers()) {
        Ok(()) => next.run(request).await,
        Err(failure) => unauthorized(failure),
    }
}

fn load_token() -> anyhow::Result<(String, TokenSource)> {
    match token_file_path() {
        Some(path) if path.is_file() => {
            let token = std::fs::read_to_string(&path)
                .with_context(|| format!("read HTTP bearer token file {}", path.display()))?;
            let token = normalize_token(&token)
                .with_context(|| format!("HTTP bearer token file is empty: {}", path.display()))?;
            Ok((token, TokenSource::File(path)))
        }
        Some(_) | None => load_env_token(),
    }
}

fn load_env_token() -> anyhow::Result<(String, TokenSource)> {
    let token = std::env::var(TOKEN_ENV)
        .with_context(|| format!("{TOKEN_ENV} is unset and token.txt is absent"))?;
    let token = normalize_token(&token).with_context(|| format!("{TOKEN_ENV} is empty"))?;
    Ok((token, TokenSource::Env))
}

fn token_file_path() -> Option<PathBuf> {
    let appdata = std::env::var_os(APPDATA_ENV)?;
    Some(PathBuf::from(appdata).join("synapse").join("token.txt"))
}

fn normalize_token(raw: &str) -> anyhow::Result<String> {
    let token = raw.trim();
    if token.is_empty() {
        bail!("empty token")
    }
    Ok(token.to_owned())
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, AuthFailure> {
    let raw = headers
        .get(header::AUTHORIZATION)
        .ok_or(AuthFailure::Missing)?
        .to_str()
        .map_err(|_| AuthFailure::Malformed)?
        .trim();
    let mut parts = raw.splitn(2, char::is_whitespace);
    let scheme = parts.next().ok_or(AuthFailure::Malformed)?;
    let token = parts.next().ok_or(AuthFailure::Malformed)?.trim();
    if !scheme.eq_ignore_ascii_case("Bearer") || token.is_empty() {
        return Err(AuthFailure::Malformed);
    }
    Ok(token)
}

fn digest_token(token: &str) -> [u8; 32] {
    let digest = Sha256::digest(token.as_bytes());
    let mut output = [0_u8; 32];
    output.copy_from_slice(&digest);
    output
}

fn unauthorized(failure: AuthFailure) -> Response {
    tracing::warn!(
        code = synapse_core::error_codes::HTTP_TOKEN_INVALID,
        reason = ?failure,
        "HTTP bearer token rejected"
    );
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        synapse_core::error_codes::HTTP_TOKEN_INVALID,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header};

    use super::HttpAuth;

    #[test]
    fn bearer_compare_accepts_only_exact_token() {
        let auth = HttpAuth::from_token("synapse-secret");
        assert!(auth.token_matches("synapse-secret"));
        assert!(!auth.token_matches("synapse-secreu"));
        assert!(!auth.token_matches("synapse-secret-longer"));
        assert!(!auth.token_matches(""));
    }

    #[test]
    fn bearer_compare_rejects_many_prefix_variants() {
        let correct = "a".repeat(64);
        let auth = HttpAuth::from_token(&correct);
        for index in 0..10_000 {
            let prefix_len = index % correct.len();
            let mut wrong = String::with_capacity(correct.len());
            wrong.push_str(&correct[..prefix_len]);
            wrong.push('b');
            wrong.push_str(&"c".repeat(correct.len() - prefix_len - 1));
            assert!(!auth.token_matches(&wrong), "prefix_len={prefix_len}");
        }
    }

    #[test]
    fn authorization_header_accepts_bearer_case_insensitive() {
        let auth = HttpAuth::from_token("local-token");
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer local-token"),
        );
        assert!(auth.authorize(&headers).is_ok());
    }
}
