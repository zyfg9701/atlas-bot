//! Shared OIDC + WeCom ticket helpers for atlas-bot (PC + CLI).
//!
//! Hub still validates via I1 (`Authorization: Bearer`); this crate only obtains
//! the ticket. Prefer handing Hub the **id_token** for OIDC (see [`oidc::pick_hub_bearer`]).
//! WeCom uses Hub thin exchange → short JWT (`sub=wecom:<corpId>:<userid>`).

#![forbid(unsafe_code)]

pub mod credentials;
pub mod loopback;
pub mod mock_oidc;
pub mod mock_wecom;
pub mod oidc;
pub mod pkce;
pub mod wecom;

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
pub use wecom::{
    build_wecom_authorize_request, exchange_wecom_code, hub_http_base_from_env, wecom_subject,
    ws_to_http_base, TicketProvider, WeComAuthorizeRequest, WeComClientConfig, WeComError,
    WeComExchangeResponse,
};
