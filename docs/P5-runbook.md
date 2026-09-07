# P5 runbook — VNC descriptor + attachment MVP

## Scope

See feitian-p5-acceptance.md and pangu-p5-vnc-attachment-boundary.md.

## Smoke

```bash
cargo test -p atlas-bot-hub --test p5_smoke -- --nocapture
# expect: SMOKE_OK p5 vnc+upload ...
```

## Start Hub

```bash
RUST_LOG=info cargo run -p atlas-bot-hub
# ws://127.0.0.1:7700/ws ; VNC stub http://127.0.0.1:8787/vnc-stub?agent=agt_1
```

## Scope details

- Hub `bot.vncDescriptor` is a first-class method (NOT a command name).
- Gateway stub: `http://127.0.0.1:8787/vnc-stub?agent=…`, expiresHint ~ now+5min.
- uploadAttachment via bot.command; args_too_large when args JSON > 3 MiB.
- attachUpload stub: valid uploadId -> path; invalid -> attachment_not_found.
- Cold status/roster/offbox never wake gateway incorrectly.
- No real noVNC, IdP, readAttachment*, or groups.

## PC click-path

1. Connect + hello (capabilities include bot.vncDescriptor).
2. Open desktop -> opens stub page; expiresHint visible.
3. Pick small file (<1.5 MiB) -> uploadAttachment shows path.
4. attachUpload (last id); closed-set errors in Failure banner.

## Known limitations

- VNC: placeholder HTML only (`ATLAS_VNC_STUB_BASE` overrides).
- Attachments: temp stub disk; attachUpload maps prior uploadId only.
- CLI gateway may return gateway/unknown-method for VNC/upload (use stub).
