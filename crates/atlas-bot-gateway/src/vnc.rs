//! D1 VNC probe + mint helpers (RFB handshake, honest stub degrade).
//!
//! Wire is unchanged: `bot.vncDescriptor` → `{ vncUrl, expiresHint }`.
//! Token URLs are minted only when `ATLAS_VNC_MODE=proxy` **and** the
//! upstream speaks RFB. Probe failure / missing upstream → stub page
//! (never claims a live desktop).

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    now_ms, urlencoding_lite, GatewayError, VncMode, VncTokenRecord, DEFAULT_VNC_TOKEN_TTL_MS,
};

/// Default TCP+banner timeout for [`probe_rfb_upstream`].
pub const DEFAULT_VNC_PROBE_TIMEOUT_MS: u64 = 800;

/// Env: override probe timeout milliseconds (`ATLAS_VNC_PROBE_TIMEOUT_MS`).
pub const ENV_VNC_PROBE_TIMEOUT_MS: &str = "ATLAS_VNC_PROBE_TIMEOUT_MS";

/// Result of an RFB probe against `ATLAS_VNC_UPSTREAM`.
#[derive(Debug, Clone)]
pub struct RfbProbe {
    pub healthy: bool,
    /// e.g. `003.008` when the server sent `RFB 003.008\n`.
    pub version: Option<String>,
    pub detail: String,
}

impl RfbProbe {
    pub fn fail(detail: impl Into<String>) -> Self {
        Self {
            healthy: false,
            version: None,
            detail: detail.into(),
        }
    }
}

fn probe_timeout() -> Duration {
    let ms = std::env::var(ENV_VNC_PROBE_TIMEOUT_MS)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_VNC_PROBE_TIMEOUT_MS);
    Duration::from_millis(ms)
}

/// Parse `host:port` or `[ipv6]:port` into (host, port).
pub fn parse_vnc_upstream(upstream: &str) -> Result<(String, u16), String> {
    let s = upstream.trim();
    if s.is_empty() {
        return Err("empty upstream".into());
    }
    if let Some(rest) = s.strip_prefix('[') {
        let (host, port_s) = rest
            .split_once("]:")
            .ok_or_else(|| format!("invalid IPv6 upstream {s}"))?;
        let port: u16 = port_s
            .parse()
            .map_err(|_| format!("invalid port in {s}"))?;
        return Ok((host.to_string(), port));
    }
    let (host, port_s) = s
        .rsplit_once(':')
        .ok_or_else(|| format!("upstream must be host:port, got {s}"))?;
    if host.is_empty() {
        return Err(format!("missing host in {s}"));
    }
    let port: u16 = port_s
        .parse()
        .map_err(|_| format!("invalid port in {s}"))?;
    Ok((host.to_string(), port))
}

/// TCP connect + read RFB version banner (`RFB xxx.yyy\\n`).
///
/// A bare TCP accept without the banner is **not** healthy (degrade-to-stub).
pub async fn probe_rfb_upstream(upstream: &str) -> RfbProbe {
    let (host, port) = match parse_vnc_upstream(upstream) {
        Ok(hp) => hp,
        Err(e) => return RfbProbe::fail(e),
    };
    let timeout = probe_timeout();
    match tokio::time::timeout(timeout, probe_rfb_inner(&host, port)).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => RfbProbe::fail(e),
        Err(_) => RfbProbe::fail(format!(
            "probe timeout after {}ms to {host}:{port}",
            timeout.as_millis()
        )),
    }
}

async fn probe_rfb_inner(host: &str, port: u16) -> Result<RfbProbe, String> {
    let mut stream = TcpStream::connect((host, port))
        .await
        .map_err(|e| format!("connect {host}:{port}: {e}"))?;
    stream
        .set_nodelay(true)
        .map_err(|e| format!("nodelay: {e}"))?;
    let mut buf = [0u8; 12];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("read RFB banner from {host}:{port}: {e}"))?;
    if !buf.starts_with(b"RFB ") || buf[11] != b'\n' {
        return Ok(RfbProbe::fail(format!(
            "not an RFB banner from {host}:{port}: {:?}",
            String::from_utf8_lossy(&buf)
        )));
    }
    let version = String::from_utf8_lossy(&buf[4..11]).trim().to_string();
    Ok(RfbProbe {
        healthy: true,
        version: Some(version.clone()),
        detail: format!("RFB {version} at {host}:{port}"),
    })
}

/// Probe (when proxy+upstream) then mint. Unhealthy → stub URL, no token.
pub async fn mint_vnc_descriptor_probed(
    agent_id: &str,
    public_base: &str,
    vnc_mode: VncMode,
    vnc_upstream: Option<&str>,
    tokens: &tokio::sync::RwLock<HashMap<String, VncTokenRecord>>,
) -> Result<Value, GatewayError> {
    let probe = match (vnc_mode, vnc_upstream) {
        (VncMode::Proxy, Some(up)) => Some(probe_rfb_upstream(up).await),
        _ => None,
    };
    let healthy = probe.as_ref().is_some_and(|p| p.healthy);
    if vnc_mode == VncMode::Proxy && vnc_upstream.is_some() && !healthy {
        warn!(
            %agent_id,
            upstream = vnc_upstream.unwrap_or(""),
            detail = probe.as_ref().map(|p| p.detail.as_str()).unwrap_or("no probe"),
            "VNC proxy probe failed — degrade-to-stub (not claiming real desktop)"
        );
    } else if healthy {
        info!(
            %agent_id,
            upstream = vnc_upstream.unwrap_or(""),
            version = probe.as_ref().and_then(|p| p.version.as_deref()).unwrap_or("?"),
            "VNC proxy probe OK — minting token URL"
        );
    }
    let version = probe.as_ref().and_then(|p| p.version.clone());
    let mut map = tokens.write().await;
    mint_vnc_descriptor_value(
        agent_id,
        public_base,
        vnc_mode,
        vnc_upstream,
        Some(&mut map),
        healthy,
        version,
    )
}

/// Shared VNC descriptor mint (P5 + R2 Box + D1). Token URL only when
/// proxy + upstream + `upstream_healthy`; otherwise stub.
pub fn mint_vnc_descriptor_value(
    agent_id: &str,
    public_base: &str,
    vnc_mode: VncMode,
    vnc_upstream: Option<&str>,
    tokens: Option<&mut HashMap<String, VncTokenRecord>>,
    upstream_healthy: bool,
    rfb_version: Option<String>,
) -> Result<Value, GatewayError> {
    if agent_id.trim().is_empty() {
        return Err(GatewayError::InvalidArgs("missing agentId".into()));
    }
    let base = public_base.trim_end_matches('/');
    let expires_hint = now_ms() + DEFAULT_VNC_TOKEN_TTL_MS;
    let use_proxy =
        vnc_mode == VncMode::Proxy && vnc_upstream.is_some() && tokens.is_some() && upstream_healthy;
    let vnc_url = if use_proxy {
        let upstream = vnc_upstream.unwrap().to_string();
        let token = format!("tok_{}", Uuid::new_v4().simple());
        if let Some(map) = tokens {
            map.retain(|_, t| t.expires_at_ms > now_ms());
            map.insert(
                token.clone(),
                VncTokenRecord {
                    agent_id: agent_id.to_string(),
                    expires_at_ms: expires_hint,
                    upstream,
                    rfb_version,
                },
            );
        }
        format!("{base}/vnc/{token}/")
    } else {
        // stub mode, or proxy without upstream / unhealthy probe → stub
        format!("{base}/vnc-stub?agent={}", urlencoding_lite(agent_id))
    };
    Ok(json!({
        "vncUrl": vnc_url,
        "expiresHint": expires_hint,
    }))
}

/// Loopback RFB 3.8 mock (security None). For tests / Evidence — not a desktop.
pub async fn spawn_loopback_mock_rfb() -> std::io::Result<(tokio::task::JoinHandle<()>, String)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let _ = speak_mock_rfb(&mut sock).await;
            });
        }
    });
    Ok((handle, format!("{addr}")))
}

async fn speak_mock_rfb(sock: &mut TcpStream) -> std::io::Result<()> {
    sock.write_all(b"RFB 003.008\n").await?;
    let mut ver = [0u8; 12];
    // Client may only probe the banner (mint path) — don't fail the accept loop.
    if sock.read_exact(&mut ver).await.is_err() {
        return Ok(());
    }
    // 1 security type: None (1)
    sock.write_all(&[1u8, 1u8]).await?;
    let mut sel = [0u8; 1];
    if sock.read_exact(&mut sel).await.is_err() {
        return Ok(());
    }
    // SecurityResult OK
    sock.write_all(&[0, 0, 0, 0]).await?;
    let mut client_init = [0u8; 1];
    if sock.read_exact(&mut client_init).await.is_err() {
        return Ok(());
    }
    // ServerInit: 64x48, 32bpp LE, name "atlas-mock"
    let name = b"atlas-mock";
    let mut si = Vec::with_capacity(24 + 4 + name.len());
    si.extend_from_slice(&64u16.to_be_bytes());
    si.extend_from_slice(&48u16.to_be_bytes());
    si.push(32); // bits-per-pixel
    si.push(24); // depth
    si.push(0); // big-endian-flag
    si.push(1); // true-colour
    si.extend_from_slice(&255u16.to_be_bytes()); // r max
    si.extend_from_slice(&255u16.to_be_bytes());
    si.extend_from_slice(&255u16.to_be_bytes());
    si.push(16); // r shift
    si.push(8);
    si.push(0);
    si.extend_from_slice(&[0, 0, 0]); // padding
    si.extend_from_slice(&(name.len() as u32).to_be_bytes());
    si.extend_from_slice(name);
    sock.write_all(&si).await?;
    // Drain a bit so a pointer-event client doesn't RST immediately.
    let mut rest = [0u8; 32];
    let _ = tokio::time::timeout(Duration::from_millis(200), sock.read(&mut rest)).await;
    Ok(())
}

/// Complete RFB 3.8 None-security handshake (Evidence). Returns version string.
pub async fn rfb_handshake_none(upstream: &str) -> Result<String, String> {
    let (host, port) = parse_vnc_upstream(upstream)?;
    let timeout = probe_timeout() + Duration::from_millis(400);
    tokio::time::timeout(timeout, rfb_handshake_none_inner(&host, port))
        .await
        .map_err(|_| format!("handshake timeout to {host}:{port}"))?
}

async fn rfb_handshake_none_inner(host: &str, port: u16) -> Result<String, String> {
    let mut stream = TcpStream::connect((host, port))
        .await
        .map_err(|e| format!("connect {host}:{port}: {e}"))?;
    let mut buf = [0u8; 12];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("banner: {e}"))?;
    if !buf.starts_with(b"RFB ") {
        return Err(format!(
            "not RFB: {:?}",
            String::from_utf8_lossy(&buf)
        ));
    }
    let version = String::from_utf8_lossy(&buf[4..11]).trim().to_string();
    stream
        .write_all(b"RFB 003.008\n")
        .await
        .map_err(|e| format!("client version: {e}"))?;
    let mut n = [0u8; 1];
    stream
        .read_exact(&mut n)
        .await
        .map_err(|e| format!("sec count: {e}"))?;
    if n[0] == 0 {
        return Err("server listed zero security types".into());
    }
    let mut types = vec![0u8; n[0] as usize];
    stream
        .read_exact(&mut types)
        .await
        .map_err(|e| format!("sec types: {e}"))?;
    if !types.contains(&1) {
        return Err(format!("no security-type None in {types:?}"));
    }
    stream
        .write_all(&[1u8])
        .await
        .map_err(|e| format!("select None: {e}"))?;
    let mut result = [0u8; 4];
    stream
        .read_exact(&mut result)
        .await
        .map_err(|e| format!("SecurityResult: {e}"))?;
    if result != [0, 0, 0, 0] {
        return Err(format!("SecurityResult failed {result:?}"));
    }
    stream
        .write_all(&[1u8])
        .await
        .map_err(|e| format!("ClientInit: {e}"))?;
    let mut si = [0u8; 24];
    stream
        .read_exact(&mut si)
        .await
        .map_err(|e| format!("ServerInit: {e}"))?;
    let w = u16::from_be_bytes([si[0], si[1]]);
    let h = u16::from_be_bytes([si[2], si[3]]);
    Ok(format!("RFB {version} ServerInit {w}x{h}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_host_port_v4_and_v6() {
        assert_eq!(
            parse_vnc_upstream("127.0.0.1:5900").unwrap(),
            ("127.0.0.1".into(), 5900)
        );
        assert_eq!(
            parse_vnc_upstream("[::1]:5901").unwrap(),
            ("::1".into(), 5901)
        );
        assert!(parse_vnc_upstream("no-port").is_err());
    }

    #[tokio::test]
    async fn probe_refuses_closed_port() {
        let p = probe_rfb_upstream("127.0.0.1:1").await;
        assert!(!p.healthy, "{p:?}");
    }

    #[tokio::test]
    async fn probe_and_handshake_mock_rfb() {
        let (_h, up) = spawn_loopback_mock_rfb().await.unwrap();
        let p = probe_rfb_upstream(&up).await;
        assert!(p.healthy, "{p:?}");
        assert_eq!(p.version.as_deref(), Some("003.008"));
        let hs = rfb_handshake_none(&up).await.unwrap();
        assert!(hs.contains("RFB 003.008"), "{hs}");
        assert!(hs.contains("64x48"), "{hs}");
    }

    #[tokio::test]
    async fn probe_rejects_non_rfb_tcp() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut s, _)) = listener.accept().await {
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await;
            }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let p = probe_rfb_upstream(&addr.to_string()).await;
        assert!(!p.healthy, "HTTP must not count as RFB: {p:?}");
    }
}
