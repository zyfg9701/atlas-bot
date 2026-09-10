//! Standalone mock WeCom for local W1 smoke.
//!
//! ```bash
//! cargo run -q -p atlas-bot-auth-client --bin mock-wecom
//! ```

use atlas_bot_auth_client::mock_wecom::{
    start_mock_wecom, MOCK_AGENT_ID, MOCK_CORP_ID, MOCK_SECRET, MOCK_USERID,
};

#[tokio::main]
async fn main() {
    let mock = start_mock_wecom().await;
    println!("mock-wecom listening on {}", mock.base_url());
    println!("corp_id={MOCK_CORP_ID}");
    println!("agent_id={MOCK_AGENT_ID}");
    println!("default_userid={MOCK_USERID}");
    println!();
    println!("export ATLAS_WECOM_CORP_ID={MOCK_CORP_ID}");
    println!("export ATLAS_WECOM_AGENT_ID={MOCK_AGENT_ID}");
    println!("export ATLAS_WECOM_SECRET={MOCK_SECRET}");
    println!("export ATLAS_WECOM_API_BASE={}", mock.base_url());
    println!("export ATLAS_WECOM_AUTHORIZE_BASE={}", mock.base_url());
    println!("export ATLAS_TICKET_PROVIDER=wecom");
    println!("# Hub also needs ATLAS_AUTH_MODE=static + ATLAS_AUTH_JWT_SECRET");
    println!();
    println!("Ctrl+C to stop.");
    std::future::pending::<()>().await;
}
