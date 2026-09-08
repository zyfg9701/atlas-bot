//! Standalone mock OIDC for local I2 smoke.
//!
//! ```bash
//! cargo run -q -p atlas-bot-auth-client --bin mock-oidc
//! # prints issuer / JWKS / env exports
//! ```

use atlas_bot_auth_client::mock_oidc::{start_mock_oidc, MOCK_AUD, MOCK_SECRET, MOCK_SUB};

#[tokio::main]
async fn main() {
    let mock = start_mock_oidc().await;
    println!("mock-oidc listening on {}", mock.base_url());
    println!("issuer={}", mock.issuer);
    println!("default_sub={MOCK_SUB}");
    println!("audience={MOCK_AUD}");
    println!();
    println!("export ATLAS_OIDC_ISSUER={}", mock.issuer);
    println!("export ATLAS_OIDC_CLIENT_ID=atlas-bot-cli");
    println!("export ATLAS_OIDC_AUDIENCE={MOCK_AUD}");
    println!("export ATLAS_AUTH_MODE=oidc");
    println!("export ATLAS_OIDC_ISSUER={}", mock.issuer);
    println!(
        "export ATLAS_OIDC_JWKS_JSON='{}'",
        mock.jwks_json().replace('\'', "'\\''")
    );
    println!("# secret (for debugging): {MOCK_SECRET}");
    println!();
    println!("Ctrl+C to stop.");
    // Park forever.
    std::future::pending::<()>().await;
}
