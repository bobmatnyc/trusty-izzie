//! Google OAuth2 authorisation flow.

use anyhow::{Context, Result};
use serde::Deserialize;

/// Manages Google OAuth2 tokens for the Gmail API.
pub struct GoogleAuthClient {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
}

/// Raw JSON shape returned by the Google token endpoint.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    scope: Option<String>,
}

impl GoogleAuthClient {
    /// Construct a new auth client from application credentials.
    pub fn new(client_id: String, client_secret: String, redirect_uri: String) -> Self {
        Self {
            client_id,
            client_secret,
            redirect_uri,
        }
    }

    /// Generate the Google consent URL the user must visit to grant access.
    pub fn authorization_url(&self) -> String {
        let scope = "https://www.googleapis.com/auth/gmail.readonly \
                     https://www.googleapis.com/auth/userinfo.email";
        // Use reqwest::Url to get correct percent-encoding without new deps.
        let mut url = reqwest::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
            .expect("static base URL is valid");
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", scope)
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent");
        url.into()
    }

    /// Exchange an authorisation code for access + refresh tokens.
    pub async fn exchange_code(&self, code: &str) -> Result<TokenSet> {
        let client = reqwest::Client::new();
        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
            ("redirect_uri", &self.redirect_uri),
        ];
        let resp: TokenResponse = client
            .post("https://oauth2.googleapis.com/token")
            .form(&params)
            .send()
            .await
            .context("POST to Google token endpoint failed")?
            .error_for_status()
            .context("Google token endpoint returned error status")?
            .json()
            .await
            .context("failed to deserialise token response")?;

        Ok(TokenSet {
            access_token: resp.access_token,
            refresh_token: resp.refresh_token,
            expires_in: resp.expires_in,
            scope: resp.scope.unwrap_or_default(),
        })
    }

    /// Use a refresh token to obtain a fresh access token.
    pub async fn refresh_token(&self, refresh_token: &str) -> Result<TokenSet> {
        let client = reqwest::Client::new();
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
        ];
        let resp: TokenResponse = client
            .post("https://oauth2.googleapis.com/token")
            .form(&params)
            .send()
            .await
            .context("POST to Google token refresh endpoint failed")?
            .error_for_status()
            .context("Google token refresh endpoint returned error status")?
            .json()
            .await
            .context("failed to deserialise refresh token response")?;

        Ok(TokenSet {
            access_token: resp.access_token,
            // Refresh responses do not include a new refresh token.
            refresh_token: resp.refresh_token,
            expires_in: resp.expires_in,
            scope: resp.scope.unwrap_or_default(),
        })
    }
}

/// An OAuth2 token pair returned by the Google token endpoint.
#[derive(Debug, Clone)]
pub struct TokenSet {
    /// Short-lived access token for API calls.
    pub access_token: String,
    /// Long-lived refresh token (only present on initial exchange).
    pub refresh_token: Option<String>,
    /// Token lifetime in seconds.
    pub expires_in: u64,
    /// Scopes granted.
    pub scope: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_client() -> GoogleAuthClient {
        GoogleAuthClient::new(
            "test-client-id".to_string(),
            "test-client-secret".to_string(),
            "http://localhost:8080/callback".to_string(),
        )
    }

    #[test]
    fn test_authorization_url_contains_required_params() {
        let client = make_client();
        let url = client.authorization_url();

        assert!(
            url.contains("response_type=code"),
            "missing response_type=code"
        );
        assert!(
            url.contains("access_type=offline"),
            "missing access_type=offline"
        );
        assert!(url.contains("prompt=consent"), "missing prompt=consent");
        assert!(
            url.contains("gmail.readonly"),
            "missing gmail.readonly scope"
        );
        assert!(
            url.contains("userinfo.email"),
            "missing userinfo.email scope"
        );
        assert!(
            url.starts_with("https://accounts.google.com/o/oauth2/v2/auth"),
            "wrong base URL"
        );
    }

    #[test]
    fn test_authorization_url_encodes_client_id() {
        let client = make_client();
        let url = client.authorization_url();
        // client_id should appear in the URL (possibly URL-encoded)
        assert!(url.contains("test-client-id"), "client_id missing from URL");
    }

    /// Network call — requires live credentials; skip in CI.
    #[tokio::test]
    #[ignore]
    async fn test_exchange_code_network() {
        let client = make_client();
        let _token_set = client.exchange_code("dummy-code").await.unwrap();
    }
}
