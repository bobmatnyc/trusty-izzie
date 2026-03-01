//! Google OAuth2 authorisation flow.

use anyhow::Result;

/// Manages Google OAuth2 tokens for the Gmail API.
pub struct GoogleAuthClient {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
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
        todo!("build OAuth2 authorization URL with gmail.readonly and email scopes")
    }

    /// Exchange an authorisation code for access + refresh tokens.
    pub async fn exchange_code(&self, _code: &str) -> Result<TokenSet> {
        todo!("POST to Google token endpoint and deserialise TokenSet")
    }

    /// Use a refresh token to obtain a fresh access token.
    pub async fn refresh_token(&self, _refresh_token: &str) -> Result<TokenSet> {
        todo!("POST to Google token refresh endpoint")
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
