#!/usr/bin/env bash
# D1 same-machine display stack: Xvfb + x11vnc (primary path).
# Loopback-only. Start then probe RFB then print ATLAS_VNC_UPSTREAM.
# Never documents public bind. Key/mouse via VNC client (not bot.command).
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/atlas-desktop-stack.sh <start|stop|status|probe|screenshot|cleanup>

Primary display stack (D1): Xvfb + x11vnc on loopback.

Env:
  ATLAS_DESKTOP_RUNDIR        PID/log dir (default: .atlas-desktop in cwd)
  ATLAS_DESKTOP_DISPLAY_NUM   X display number (default: 97)
  ATLAS_DESKTOP_RFB_PORT      x11vnc port (default: free loopback port)
  ATLAS_DESKTOP_GEOM          Xvfb geometry (default: 1280x720x24)
  ATLAS_BOX_WORKSPACE         Box root; xterm cwd becomes <root>/<agent>/
  ATLAS_DESKTOP_AGENT_ID      Agent id for workspace cwd (default: agt_1)

start   - Xvfb then x11vnc -localhost -nopw; writes rundir/env
stop    - kill PIDs recorded by start
status  - print pids / upstream / RFB banner
probe   - RFB handshake against recorded upstream (exit 0/1)
screenshot - xwd/import if present, else skip (RFB handshake is Evidence)
cleanup - stop + remove rundir (leak patrol)

See docs/p1-desktop-runbook.md
EOF
}

cmd="${1:-}"
if [[ -z "$cmd" || "$cmd" == "-h" || "$cmd" == "--help" ]]; then
  usage
  exit 0
fi

RUNDIR="${ATLAS_DESKTOP_RUNDIR:-.atlas-desktop}"
mkdir -p "$RUNDIR"
RUNDIR="$(cd "$RUNDIR" && pwd)"
DISPLAY_NUM="${ATLAS_DESKTOP_DISPLAY_NUM:-97}"
GEOM="${ATLAS_DESKTOP_GEOM:-1280x720x24}"
AGENT_ID="${ATLAS_DESKTOP_AGENT_ID:-agt_1}"
XVFB_PIDFILE="$RUNDIR/xvfb.pid"
VNC_PIDFILE="$RUNDIR/x11vnc.pid"
ENVFILE="$RUNDIR/env"
LOG_XVFB="$RUNDIR/xvfb.log"
LOG_VNC="$RUNDIR/x11vnc.log"
SHOT="$RUNDIR/screenshot.xwd"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "error: missing '$1' (install xvfb + x11vnc for the primary path)" >&2
    exit 2
  }
}

pid_alive() {
  local f="$1"
  [[ -f "$f" ]] || return 1
  local pid
  pid="$(cat "$f" 2>/dev/null || true)"
  [[ -n "${pid:-}" ]] && kill -0 "$pid" 2>/dev/null
}

free_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

rfb_probe() {
  local up="$1"
  python3 - "$up" <<'PY'
import socket, sys
up = sys.argv[1]
host, port = up.rsplit(":", 1)
s = socket.create_connection((host, int(port)), 1.5)
s.settimeout(1.5)
data = s.recv(12)
sys.stdout.write(repr(data) + "\n")
ok = data.startswith(b"RFB ") and data.endswith(b"\n")
sys.exit(0 if ok else 1)
PY
}

write_env() {
  local port="$1"
  cat > "$ENVFILE" <<EOF
ATLAS_VNC_UPSTREAM=127.0.0.1:${port}
ATLAS_VNC_MODE=proxy
DISPLAY=:${DISPLAY_NUM}
ATLAS_DESKTOP_DISPLAY_NUM=${DISPLAY_NUM}
ATLAS_DESKTOP_RFB_PORT=${port}
ATLAS_DESKTOP_RUNDIR=${RUNDIR}
EOF
}

do_stop() {
  if [[ -f "$RUNDIR/xterm.pid" ]] && pid_alive "$RUNDIR/xterm.pid"; then
    kill "$(cat "$RUNDIR/xterm.pid")" 2>/dev/null || true
  fi
  if pid_alive "$VNC_PIDFILE"; then
    kill "$(cat "$VNC_PIDFILE")" 2>/dev/null || true
    sleep 0.15
    pid_alive "$VNC_PIDFILE" && kill -9 "$(cat "$VNC_PIDFILE")" 2>/dev/null || true
  fi
  if pid_alive "$XVFB_PIDFILE"; then
    kill "$(cat "$XVFB_PIDFILE")" 2>/dev/null || true
    sleep 0.15
    pid_alive "$XVFB_PIDFILE" && kill -9 "$(cat "$XVFB_PIDFILE")" 2>/dev/null || true
  fi
  rm -f "$XVFB_PIDFILE" "$VNC_PIDFILE" "$RUNDIR/xterm.pid"
  echo "stopped display :${DISPLAY_NUM}"
}

do_start() {
  need Xvfb
  need x11vnc
  if pid_alive "$XVFB_PIDFILE" || pid_alive "$VNC_PIDFILE"; then
    echo "error: stack already running (see $RUNDIR). Use stop first." >&2
    exit 3
  fi
  if [[ -S "/tmp/.X11-unix/X${DISPLAY_NUM}" ]]; then
    echo "error: DISPLAY :${DISPLAY_NUM} already has a socket. Set ATLAS_DESKTOP_DISPLAY_NUM." >&2
    exit 3
  fi
  local port="${ATLAS_DESKTOP_RFB_PORT:-}"
  if [[ -z "$port" ]]; then
    port="$(free_port)"
  fi

  Xvfb ":${DISPLAY_NUM}" -screen 0 "$GEOM" -ac +extension RANDR \
    >"$LOG_XVFB" 2>&1 &
  echo $! > "$XVFB_PIDFILE"

  local i=0
  while [[ $i -lt 50 ]]; do
    if [[ -S "/tmp/.X11-unix/X${DISPLAY_NUM}" ]]; then
      break
    fi
    sleep 0.1
    i=$((i + 1))
  done
  if [[ ! -S "/tmp/.X11-unix/X${DISPLAY_NUM}" ]]; then
    echo "error: Xvfb did not create /tmp/.X11-unix/X${DISPLAY_NUM}" >&2
    cat "$LOG_XVFB" >&2 || true
    do_stop
    exit 4
  fi

  if command -v xsetroot >/dev/null 2>&1; then
    DISPLAY=":${DISPLAY_NUM}" xsetroot -solid "#1a2332" || true
  fi

  local ws_dir=""
  if [[ -n "${ATLAS_BOX_WORKSPACE:-}" ]]; then
    ws_dir="${ATLAS_BOX_WORKSPACE%/}/${AGENT_ID}"
    mkdir -p "$ws_dir"
  fi
  if command -v xterm >/dev/null 2>&1 && [[ -n "$ws_dir" ]]; then
    DISPLAY=":${DISPLAY_NUM}" xterm -geometry 80x24+20+20 -e bash -lc "cd $(printf %q "$ws_dir"); exec bash" \
      >/dev/null 2>&1 &
    echo $! > "$RUNDIR/xterm.pid" || true
  fi

  # This x11vnc has no -pid; background ourselves and record $!.
  x11vnc -display ":${DISPLAY_NUM}" -rfbport "$port" -localhost -nopw -forever -shared \
    -noxdamage -o "$LOG_VNC" \
    >/dev/null 2>&1 &
  echo $! > "$VNC_PIDFILE"
  sleep 0.4
  if ! pid_alive "$VNC_PIDFILE"; then
      echo "error: x11vnc failed to start" >&2
      cat "$LOG_VNC" >&2 || true
      do_stop
      exit 5
  fi

  write_env "$port"
  local up="127.0.0.1:${port}"
  if ! rfb_probe "$up"; then
    echo "error: RFB probe failed against $up" >&2
    cat "$LOG_VNC" >&2 || true
    do_stop
    exit 6
  fi
  echo "started Xvfb+x11vnc  DISPLAY=:${DISPLAY_NUM}  ATLAS_VNC_UPSTREAM=${up}"
  echo "env file: $ENVFILE"
  echo "export ATLAS_VNC_MODE=proxy ATLAS_VNC_UPSTREAM=${up}"
}

do_status() {
  local xv="dead" vn="dead"
  pid_alive "$XVFB_PIDFILE" && xv="pid $(cat "$XVFB_PIDFILE")"
  pid_alive "$VNC_PIDFILE" && vn="pid $(cat "$VNC_PIDFILE")"
  echo "xvfb: $xv"
  echo "x11vnc: $vn"
  if [[ -f "$ENVFILE" ]]; then
    cat "$ENVFILE"
  fi
}

do_probe() {
  if [[ ! -f "$ENVFILE" ]]; then
    echo "error: no $ENVFILE (start first)" >&2
    exit 1
  fi
  # shellcheck disable=SC1090
  source "$ENVFILE"
  rfb_probe "${ATLAS_VNC_UPSTREAM}"
  echo "RFB probe OK ${ATLAS_VNC_UPSTREAM}"
}

do_screenshot() {
  if [[ ! -f "$ENVFILE" ]]; then
    echo "error: no $ENVFILE (start first)" >&2
    exit 1
  fi
  # shellcheck disable=SC1090
  source "$ENVFILE"
  if command -v xwd >/dev/null 2>&1; then
    DISPLAY="${DISPLAY}" xwd -root -out "$SHOT"
    echo "wrote $SHOT"
    return 0
  fi
  if command -v import >/dev/null 2>&1; then
    DISPLAY="${DISPLAY}" import -window root "$RUNDIR/screenshot.png"
    echo "wrote $RUNDIR/screenshot.png"
    return 0
  fi
  echo "screenshot skipped: no xwd/import (RFB handshake remains Evidence)"
  return 0
}

case "$cmd" in
  start) do_start ;;
  stop) do_stop ;;
  status) do_status ;;
  probe) do_probe ;;
  screenshot) do_screenshot ;;
  cleanup)
    do_stop
    rm -rf "$RUNDIR"
    echo "cleaned $RUNDIR"
    ;;
  *)
    usage
    exit 1
    ;;
esac
