use axum::{
    extract::{FromRequestParts, Path},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};

pub struct Id(pub String);

pub struct User(pub String);

pub fn is_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

pub fn is_user(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().any(|b| b.is_ascii_alphanumeric())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

pub fn is_uri(uri: &str, kinds: &[&str]) -> bool {
    let mut parts = uri.splitn(3, ':');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some("spotify"), Some(kind), Some(id)) if kinds.contains(&kind) && is_id(id)
    )
}

pub fn is_liked_uri(uri: &str) -> bool {
    uri.strip_prefix("spotify:user:")
        .and_then(|rest| rest.strip_suffix(":collection"))
        .is_some_and(is_user)
}

async fn segment<S: Send + Sync>(parts: &mut Parts, state: &S, valid: fn(&str) -> bool) -> Result<String, Response> {
    let Path(value) = Path::<String>::from_request_parts(parts, state).await.map_err(IntoResponse::into_response)?;
    if valid(&value) { Ok(value) } else { Err((StatusCode::BAD_REQUEST, "invalid id").into_response()) }
}

impl<S: Send + Sync> FromRequestParts<S> for Id {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        segment(parts, state, is_id).await.map(Id)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for User {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        segment(parts, state, is_user).await.map(User)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert!(is_id("3TVXtAsR1Inumwj472S9r4"));
        assert!(!is_id(""));
        assert!(!is_id("../me"));
        assert!(!is_id(&"a".repeat(65)));
    }

    #[test]
    fn users() {
        assert!(is_user("exampleuser123"));
        assert!(is_user("john.doe-1"));
        assert!(!is_user(".."));
        assert!(!is_user("."));
        assert!(!is_user("a/b"));
    }

    #[test]
    fn uris() {
        assert!(is_uri("spotify:track:0s3nnoMeVWz3989MkNQiRf", &["track"]));
        assert!(!is_uri("spotify:track:0s3nnoMeVWz3989MkNQiRf", &["album"]));
        assert!(!is_uri("javascript:alert(1)", &["track"]));
        assert!(!is_uri("spotify:track:../../x", &["track"]));
        assert!(is_liked_uri("spotify:user:exampleuser123:collection"));
        assert!(!is_liked_uri("spotify:user:..:collection"));
    }
}
