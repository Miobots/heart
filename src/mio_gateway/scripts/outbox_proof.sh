#!/usr/bin/env bash
# H3.3 — prove the outbox: an outage in the middle of a drive loses nothing, trims nothing early,
# and delivers everything in order with the time it happened.
#
# Headless Gazebo + Nav2 + the safety supervisor + this gateway + the real Brain, with wiretap.ts
# between the gateway and the Brain writing down every message. Runs on its own ROS domain and
# Gazebo partition. Needs miobots-brain and miobots-protocol checked out beside this repo, the
# workspace built (./build.sh), and the gateway built with `cargo build --features ros`.
#
#   src/mio_gateway/scripts/outbox_proof.sh [output dir]
#
# Exits 0 only if every check passes. Logs, the tap and the outbox snapshots stay in the output dir.

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
BRAIN_DIR="${BRAIN_DIR:-$REPO/../miobots-brain}"
WORKSPACE="${WORKSPACE:-$REPO}"                       # where install/setup.bash is
GATEWAY="${GATEWAY:-$REPO/src/mio_gateway/target/debug/mio_gateway}"
OUT="${1:-$(mktemp -d /tmp/outbox-proof.XXXX)}"
mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd)"

source /opt/ros/lyrical/setup.bash
source "$WORKSPACE/install/setup.bash"
set -u
export ROS_DOMAIN_ID="${ROS_DOMAIN_ID:-83}" GZ_PARTITION="outbox-proof-$$"
export DEV_TOKEN=outbox-proof-token DEVICE_ID=heart-proof
BRAIN_PORT=8297 TAP_PORT=8298
export BRAIN_URL="ws://127.0.0.1:$TAP_PORT/ws"
OUTBOX="$OUT/outbox" TAP="$OUT/tap.jsonl"
REGIONS="$(ros2 pkg prefix mio_nav)/share/mio_nav/maps/home_arena.regions.json"
rm -rf "$OUTBOX" "$TAP" "$OUT"/*.log "$OUT/stored"

failed=0
pass() { echo "PASS  $*"; }
fail() { echo "FAIL  $*"; failed=1; }
step() { echo; echo "== $(date +%T) $*"; }
# wait_for <seconds> <command...>: true once the command succeeds, false if time runs out.
wait_for() { local t=$1; shift; for ((i = 0; i < t; i++)); do "$@" && return 0; sleep 1; done; return 1; }
topics_on_disk() { python3 -I -c 'import json,sys,glob; [print(json.load(open(f))["topic"]) for f in sorted(glob.glob(sys.argv[1]+"/*.json"))]' "$OUTBOX"; }
on_disk() { topics_on_disk | grep -qx "$1"; }
tapped() { grep -q "$1" "$TAP" 2>/dev/null; }

pids=()
start() { setsid "$@" & pids+=($!); }               # each in its own group, so Ctrl-C reaches all of it
stop() { kill -INT -- "-$1" 2>/dev/null; for _ in $(seq 15); do kill -0 "$1" 2>/dev/null || return; sleep 1; done; kill -KILL -- "-$1" 2>/dev/null; }
cleanup() { for p in "${pids[@]}"; do stop "$p"; done; wait 2>/dev/null; ros2 daemon stop >/dev/null 2>&1; }
trap cleanup EXIT

start_brain() { (cd "$BRAIN_DIR" && PORT=$BRAIN_PORT exec setsid bun run src/index.ts) >>"$OUT/brain.log" 2>&1 & BRAIN=$!; pids+=($BRAIN); }
start_gateway() { start "$GATEWAY" --ros-args -p regions_file:="$REGIONS" -p outbox_dir:="$OUTBOX" >>"$OUT/gateway.log" 2>&1; GW=${pids[-1]}; }

step "simulation, Nav2 and the supervisor (headless)"
start ros2 launch mio_sim sim.launch.py headless:=true >"$OUT/sim.log" 2>&1
sleep 8
start ros2 launch mio_nav navigation.launch.py use_rviz:=false >"$OUT/nav.log" 2>&1
if wait_for 120 bash -c 'ros2 lifecycle get /bt_navigator 2>/dev/null | grep -q active'; then pass "Nav2 is up"
else fail "Nav2 never came up (HANDOFF: the controller_server start-up flake) — run again"; exit 1; fi

step "the Brain, the tap, and the gateway"
start bun "$HERE/wiretap.ts" $TAP_PORT "ws://127.0.0.1:$BRAIN_PORT/ws" "$TAP" >"$OUT/tap.log" 2>&1
start_brain
sleep 3
start_gateway
wait_for 30 tapped '"driving":{"state":"available"}' && pass "gateway let in, driving available" || { fail "the robot never said it can drive"; exit 1; }

step "the app asks for the living room; the Brain is killed as soon as progress flows"
bun "$HERE/app_goal.ts" "ws://127.0.0.1:$BRAIN_PORT/ws" "$DEV_TOKEN" "living room" | tee -a "$OUT/app.log"
[ "${PIPESTATUS[0]}" = 0 ] && pass "goal accepted" || { fail "goal not accepted"; exit 1; }
wait_for 20 tapped '"topic":"nav.feedback"' && pass "progress reached the Brain" || fail "no progress before the outage"
kill -KILL -- "-$BRAIN"; echo "Brain killed at $(date +%T.%N | cut -c1-12)"

step "the outage: the robot finishes the drive, then is tipped over"
wait_for 90 on_disk nav.result && pass "nav.result on disk — the drive ended with no Brain" || fail "no nav.result on disk"
gz service -s /world/home_arena/set_pose --reqtype gz.msgs.Pose --reptype gz.msgs.Boolean --timeout 3000 \
  --req 'name: "mio", position: {z: 0.3}, orientation: {x: 0.5646, w: 0.8253}' >/dev/null
wait_for 20 on_disk diag.fault && pass "diag.fault on disk — the tip-over, with no Brain" || fail "no diag.fault on disk"
tapped '"topic":"nav.result"' && fail "nav.result crossed the wire during the outage?" || pass "nothing crossed while the Brain was down"

step "the gateway is killed and restarted, still with no Brain"
mkdir -p "$OUT/stored"; cp "$OUTBOX"/*.json "$OUT/stored/"
kill -KILL -- "-$GW"
start_gateway
if wait_for 10 bash -c "[ \$(grep -c 'still waiting' '$OUT/gateway.log') -ge 2 ]"; then
  found=$(grep 'still waiting' "$OUT/gateway.log" | tail -1 | grep -oE '[0-9]+ event')
  [ "${found% event}" = "$(ls "$OUT"/stored/*.json | wc -l)" ] && pass "the restarted gateway found all ${found}s on disk" || fail "the restarted gateway found ${found}(s), not $(ls "$OUT"/stored/*.json | wc -l)"
else fail "the restarted gateway never said what was waiting"; fi
sleep 8
for f in "$OUT"/stored/*.json; do [ -e "$OUTBOX/$(basename "$f")" ] || fail "$(basename "$f") left the outbox with no Brain to confirm it"; done
pass "after a restart and 8 s more with no Brain, every stored event is still on disk: $(topics_on_disk | tr '\n' ' ')"

step "the Brain comes back"
cp "$OUTBOX"/*.json "$OUT/stored/"    # anything the restarted gateway added meanwhile
start_brain
wait_for 40 bash -c "[ -z \"\$(ls '$OUTBOX'/*.json 2>/dev/null)\" ]" && pass "the outbox emptied" || fail "events still on disk: $(topics_on_disk | tr '\n' ' ')"

step "what crossed the wire"
python3 -I - "$OUT/stored" "$TAP" <<'EOF' || failed=1
import json, sys, glob
stored = [json.load(open(f)) for f in sorted(glob.glob(sys.argv[1] + "/*.json"))]
tap = [json.loads(l) for l in open(sys.argv[2])]
ids = [e["msg_id"] for e in stored]
sent = [t for t in tap if t["dir"] == "heart→brain" and t["msg_id"] in ids]
receipts = [m for t in tap if t["dir"] == "brain→heart" and t["topic"] == "sys.receipt" for m in t["payload"]["msg_ids"]]
ok = True
def check(good, what):
    global ok
    print(("PASS  " if good else "FAIL  ") + what); ok &= good
for e in stored:
    print(f"      stored  {e['topic']:<11} {e['msg_id']}  happened {e['t_wall_ms']}")
for t in sent:
    print(f"      sent    {t['topic']:<11} {t['msg_id']}  t_wall_ms {t['t_wall_ms']}  seq {t['seq']}  crossed {t['at']}")
check([t["msg_id"] for t in sent] == ids, "every stored event crossed, once each, in the order it happened")
check(all(t["t_wall_ms"] == e["t_wall_ms"] for t, e in zip(sent, stored)), "each carries the time it happened, not the time it was sent")
check(all(t["at"] - t["t_wall_ms"] > 5000 for t in sent), "and that time is well before it crossed (it waited out the outage)")
check(sorted(set(receipts) & set(ids)) == sorted(ids), "the Brain confirmed every one")
sys.exit(0 if ok else 1)
EOF

echo
[ $failed = 0 ] && echo "H3.3: ALL CHECKS PASSED — output in $OUT" || echo "H3.3: SOME CHECKS FAILED — output in $OUT"
exit $failed
