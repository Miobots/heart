// Plays the app for one request: connects to the Brain as Synapse and asks it to drive the robot
// to a named place, the way a tap on the map does (H3.3). Prints the robot's answer and exits.
//
//   bun app_goal.ts <brain ws url> <dev token> <place>
//
// Uses the shared codec from miobots-protocol, checked out beside this repo (as in CI).

import { DeviceRole, Kind, Topics, decode, encode, newEnvelope, SequenceCounter } from "../../../../miobots-protocol/src/index.ts";

const [url, token, place] = Bun.argv.slice(2);
if (!url || !token || !place) {
  console.error("usage: bun app_goal.ts <brain ws url> <dev token> <place>");
  process.exit(2);
}

const seq = new SequenceCounter();
const ws = new WebSocket(url);
const fail = (why: string) => {
  console.error(`app_goal: ${why}`);
  process.exit(1);
};
setTimeout(() => fail("no answer within 20 s"), 20_000);

ws.onopen = () => {
  const payload = { device_id: "h33-app", token, protocol_version: 1, role: DeviceRole.SYNAPSE };
  ws.send(encode(newEnvelope({ kind: Kind.CMD, topic: Topics.SYS_HELLO, payload, seq })));
};
ws.onmessage = (ev) => {
  const env = decode(String(ev.data)) as { kind: string; topic: string; payload: Record<string, unknown> };
  if (env.topic === Topics.SYS_WELCOME) {
    if (env.payload.accepted !== true) fail(`the Brain refused the app: ${JSON.stringify(env.payload)}`);
    const goal = newEnvelope({
      kind: Kind.CMD,
      topic: Topics.NAV_GOTO,
      payload: { goal_id: `h33-${Date.now()}`, region: place },
      seq,
    });
    ws.send(encode({ ...goal, idem_key: `h33-${goal.msg_id}`, expires_at: Date.now() + 30_000 }));
  } else if (env.kind === Kind.ACK && env.topic === Topics.NAV_GOTO) {
    console.log(`app_goal: ${JSON.stringify(env.payload)}`);
    process.exit(env.payload.accepted === true ? 0 : 1);
  }
};
ws.onclose = () => fail("the Brain closed the connection");
