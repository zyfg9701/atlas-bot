//! Mint an HS256 Hub JWT for static / oidc-mock smoke.
//!
//! ```bash
//! ATLAS_AUTH_JWT_SECRET=dev-secret cargo run -p atlas-bot-hub --bin mint-jwt -- \
//!   --sub user_alice --ttl 3600
//! ```

use std::env;

fn usage() -> ! {
    eprintln!(
        "usage: mint-jwt --sub <subject> [--ttl SECS] [--iss ISSUER] [--aud AUDIENCE] [--secret SECRET]\n\
         secret from --secret or ATLAS_AUTH_JWT_SECRET"
    );
    std::process::exit(2);
}

fn main() {
    let mut args = env::args().skip(1);
    let mut sub: Option<String> = None;
    let mut ttl: i64 = 3600;
    let mut iss: Option<String> = None;
    let mut aud: Option<String> = None;
    let mut secret: Option<String> = None;

    while let Some(a) = args.next() {
        match a.as_str() {
            "--sub" => sub = args.next(),
            "--ttl" => {
                ttl = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| usage())
            }
            "--iss" => iss = args.next(),
            "--aud" => aud = args.next(),
            "--secret" => secret = args.next(),
            "-h" | "--help" => usage(),
            other => {
                eprintln!("unknown arg: {other}");
                usage();
            }
        }
    }

    let sub = sub.unwrap_or_else(|| usage());
    let secret = secret
        .or_else(|| env::var("ATLAS_AUTH_JWT_SECRET").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            eprintln!("missing --secret / ATLAS_AUTH_JWT_SECRET");
            usage();
        });

    match atlas_bot_hub::auth::mint_hs256(
        &secret,
        &sub,
        ttl,
        iss.as_deref(),
        aud.as_deref(),
    ) {
        Ok(tok) => {
            println!("{tok}");
        }
        Err(e) => {
            eprintln!("mint failed: {e}");
            std::process::exit(1);
        }
    }
}
