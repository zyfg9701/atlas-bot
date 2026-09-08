//! Shared OIDC Authorization Code + PKCE helpers for atlas-bot I2.1 (PC + CLI).
//!
//! Hub still validates via I1 (`Authorization: Bearer`); this crate only obtains
//! the ticket. Prefer handing Hub the **id_token** (see [`oidc::pick_hub_bearer`]).

#![forbid(unsafe_code)]

pub mod credentials;
pub mod loopback;
pub mod mock_oidc;
pub mod oidc;
pub mod pkce;

pub use credentials::{
    config_dir, credentials_path, delete_credentials, load_bearer, load_credentials,
    save_credentials, CredentialError, StoredCredentials,
};
pub use loopback::{
    redirect_port_from_env, start_loopback, wait_for_callback, CallbackResult, LoopbackError,
};
pub use oidc::{
    build_authorize_request, exchange_code, looks_like_jwt, peek_sub, pick_hub_bearer,
    AuthorizeRequest, OidcClientConfig, OidcError, TokenResponse,
};
pub use pkce::{random_verifier, s256_challenge, PkcePair};
