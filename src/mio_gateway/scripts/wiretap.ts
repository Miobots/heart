// A WebSocket relay that writes down every message between the gateway and the Brain (H3.3).
// The gateway dials this; this dials the Brain. When the Brain is down, the gateway's connection
// is closed at once, so it backs off and redials exactly as it would against the Brain itself.
//
//   bun wiretap.ts <listen port> <brain ws url> <log file>
//
// One JSON line per message: when it crossed, which way, and the envelope fields that matter.

const [port, brainUrl, logPath] = Bun.argv.slice(2);
if (!port || !brainUrl || !logPath) {
  console.error("usage: bun wiretap.ts <listen port> <brain ws url> <log file>");
  process.exit(2);
}

const log = Bun.file(logPath).writer();
function note(dir: string, raw: string): void {
  let e: Record<string, unknown> = {};
  try {
    e = JSON.parse(raw);
  } catch {
    // written down as-is below: a message that is not JSON is worth seeing
  }
  const { kind, topic, msg_id, corr_id, seq, t_wall_ms, payload } = e;
  log.write(JSON.stringify({ at: Date.now(), dir, kind, topic, msg_id, corr_id, seq, t_wall_ms, payload, raw: e.kind ? undefined : raw }) + "\n");
  log.flush();
}

type Link = { brain?: WebSocket; waiting: string[] };

Bun.serve<Link, never>({
  port: Number(port),
  fetch(req, server) {
    return server.upgrade(req, { data: { waiting: [] } }) ? undefined : new Response("WebSocket only", { status: 400 });
  },
  websocket: {
    open(heart) {
      const brain = new WebSocket(brainUrl);
      heart.data.brain = brain;
      brain.onopen = () => {
        for (const m of heart.data.waiting) brain.send(m);
        heart.data.waiting = [];
      };
      brain.onmessage = (ev) => {
        const m = String(ev.data);
        note("brain→heart", m);
        heart.send(m);
      };
      brain.onclose = () => {
        note("tap", "the Brain's side closed");
        heart.close();
      };
    },
    message(heart, message) {
      const m = String(message);
      note("heart→brain", m);
      const brain = heart.data.brain;
      if (brain?.readyState === WebSocket.OPEN) brain.send(m);
      else heart.data.waiting.push(m);
    },
    close(heart) {
      heart.data.brain?.close();
    },
  },
});
console.log(`wiretap: ws://127.0.0.1:${port}/ws → ${brainUrl}, writing ${logPath}`);
