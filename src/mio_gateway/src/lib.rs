//! Rust side of the shared MioBots wire contract. The vectors live in miobots-protocol.

use serde_json::{Map, Value};

const MAX_BYTES: usize = 512 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct WireError {
    pub reason: &'static str,
    pub field: Option<&'static str>,
}

fn error(reason: &'static str, field: Option<&'static str>) -> WireError {
    WireError { reason, field }
}

fn invalid(field: &'static str) -> WireError {
    error("invalid_envelope", Some(field))
}

fn record(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}

fn text(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|s| !s.is_empty())
}

fn number(value: Option<&Value>) -> bool {
    value.and_then(Value::as_f64).is_some_and(f64::is_finite)
}

fn positive_int(value: Option<&Value>) -> bool {
    value.and_then(Value::as_u64).is_some_and(|n| n > 0)
}

fn pose(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_array)
        .is_some_and(|p| p.len() == 3 && p.iter().all(|n| number(Some(n))))
}

fn polygon(value: Option<&Value>) -> bool {
    value.and_then(Value::as_array).is_some_and(|points| {
        points.len() >= 3
            && points.iter().all(|point| {
                point
                    .as_array()
                    .is_some_and(|xy| xy.len() == 2 && xy.iter().all(|n| number(Some(n))))
            })
    })
}

fn optional_text(obj: &Map<String, Value>, key: &str) -> bool {
    !obj.contains_key(key) || text(obj.get(key))
}

fn optional_pose(obj: &Map<String, Value>, key: &str) -> bool {
    !obj.contains_key(key) || pose(obj.get(key))
}

fn json_value(value: &Value, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    match value {
        Value::Array(items) => {
            items.len() <= 1_024 && items.iter().all(|item| json_value(item, depth + 1))
        }
        Value::Object(fields) => {
            fields.len() <= 1_024
                && fields.iter().all(|(key, item)| {
                    !["__proto__", "prototype", "constructor"].contains(&key.as_str())
                        && json_value(item, depth + 1)
                })
        }
        _ => true,
    }
}

fn rpc_id(value: Option<&Value>) -> bool {
    value.is_some_and(|v| text(Some(v)) || number(Some(v)))
}

fn rpc_params(value: Option<&Value>) -> bool {
    value.is_none_or(|v| (v.is_object() || v.is_array()) && json_value(v, 0))
}

fn rpc_fields(payload: &Map<String, Value>, allowed: &[&str]) -> bool {
    payload.keys().all(|key| allowed.contains(&key.as_str()))
}

fn rpc_valid(topic: &str, payload: &Map<String, Value>) -> bool {
    if payload.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return false;
    }
    match topic {
        "mcp.request" => {
            rpc_fields(payload, &["jsonrpc", "id", "method", "params"])
                && rpc_id(payload.get("id"))
                && text(payload.get("method"))
                && rpc_params(payload.get("params"))
        }
        "mcp.notification" => {
            rpc_fields(payload, &["jsonrpc", "method", "params"])
                && text(payload.get("method"))
                && rpc_params(payload.get("params"))
        }
        "mcp.response" => {
            if !rpc_fields(payload, &["jsonrpc", "id", "result", "error"])
                || !rpc_id(payload.get("id"))
                || payload.contains_key("result") == payload.contains_key("error")
            {
                return false;
            }
            if let Some(result) = payload.get("result") {
                return json_value(result, 0);
            }
            payload.get("error").and_then(record).is_some_and(|err| {
                err.get("code")
                    .is_some_and(|n| n.as_i64().is_some() || n.as_u64().is_some())
                    && text(err.get("message"))
                    && err.get("data").is_none_or(|v| json_value(v, 0))
            })
        }
        _ => false,
    }
}

fn allowed_kind(topic: &str, kind: &str) -> bool {
    let kinds: &[&str] = match topic {
        "sys.hello" | "sys.approval" | "sys.killswitch" | "voice.speak" | "nav.goto"
        | "nav.cancel" | "sched.window" | "region.upsert" | "region.confirm" | "zone.upsert"
        | "mcp.request" => &["CMD", "ACK"],
        "sys.welcome" | "sys.receipt" => &["ACK"],
        "sys.heartbeat" | "nav.feedback" | "nav.result" | "sched.fired" | "obs.sighting"
        | "diag.fault" | "mcp.response" => &["EVT"],
        "sys.error" => &["ERR"],
        "state.pose" | "state.battery" | "map.snapshot" => &["TELEM"],
        "cap.manifest" => &["EVT", "QRY", "RPY"],
        "mcp.notification" => &["CMD", "EVT", "ACK"],
        _ => return true,
    };
    kinds.contains(&kind)
}

fn payload_valid(topic: &str, payload: &Map<String, Value>) -> bool {
    let get = |key| payload.get(key);
    match topic {
        "sys.error" => text(get("reason")),
        "sys.approval" => {
            text(get("approval_id"))
                && text(get("tool_name"))
                && get("args").and_then(record).is_some()
                && get("args").is_some_and(|args| json_value(args, 0))
                && number(get("expires_at"))
        }
        "sys.killswitch" => get("active").and_then(Value::as_bool).is_some(),
        "sched.window" => get("occurrences")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().all(|item| {
                    item.as_object().is_some_and(|o| {
                        text(o.get("occurrence_id"))
                            && text(o.get("reminder_id"))
                            && text(o.get("text"))
                            && number(o.get("due_time_ms"))
                    })
                })
            }),
        "sched.fired" => text(get("occurrence_id")),
        "obs.sighting" => {
            text(get("label"))
                && get("aliases")
                    .and_then(Value::as_array)
                    .is_some_and(|a| a.iter().all(|s| text(Some(s))))
                && get("confidence")
                    .and_then(Value::as_f64)
                    .is_some_and(|n| (0.0..=1.0).contains(&n))
                && pose(get("pose"))
                && optional_text(payload, "region_id")
                && optional_text(payload, "thumbnail_url")
        }
        "diag.fault" => {
            text(get("subsystem"))
                && get("code").and_then(Value::as_str).is_some_and(|s| {
                    let mut chars = s.chars();
                    chars.next().is_some_and(|c| c.is_ascii_uppercase())
                        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                })
                && matches!(
                    get("severity").and_then(Value::as_str),
                    Some("info" | "warning" | "error")
                )
                && optional_text(payload, "recovery_hint")
                && optional_pose(payload, "pose")
        }
        "map.snapshot" => map_valid(payload),
        "region.upsert" => {
            text(get("region_id"))
                && (text(get("name_en")) || text(get("name_ur")))
                && optional_text(payload, "name_en")
                && optional_text(payload, "name_ur")
                && polygon(get("polygon"))
                && get("confirmed").and_then(Value::as_bool).is_some()
        }
        "region.confirm" => text(get("region_id")),
        "zone.upsert" => text(get("zone_id")) && polygon(get("polygon")),
        "mcp.request" | "mcp.response" | "mcp.notification" => rpc_valid(topic, payload),
        _ => true,
    }
}

fn map_valid(payload: &Map<String, Value>) -> bool {
    let (Some(width), Some(height)) = (
        payload.get("width").and_then(Value::as_u64),
        payload.get("height").and_then(Value::as_u64),
    ) else {
        return false;
    };
    if width == 0
        || height == 0
        || width.saturating_mul(height) > 4_000_000
        || !payload
            .get("resolution")
            .and_then(Value::as_f64)
            .is_some_and(|n| n > 0.0)
    {
        return false;
    }
    let Some(origin) = payload.get("origin").and_then(record) else {
        return false;
    };
    if !number(origin.get("x")) || !number(origin.get("y")) {
        return false;
    }
    let Some(cells) = payload.get("cells").and_then(Value::as_array) else {
        return false;
    };
    if cells.len() % 2 != 0 {
        return false;
    }
    let mut total = 0_u64;
    for pair in cells.chunks_exact(2) {
        if !matches!(pair[0].as_u64(), Some(0..=2)) || !positive_int(Some(&pair[1])) {
            return false;
        }
        total = total.saturating_add(pair[1].as_u64().unwrap());
    }
    total == width * height
}

/// Decode one envelope and validate the same wire invariants as the TypeScript codec.
pub fn decode(raw: &[u8]) -> Result<Value, WireError> {
    if raw.len() > MAX_BYTES {
        return Err(error("oversized_payload", None));
    }
    let value: Value = serde_json::from_slice(raw).map_err(|_| error("malformed", None))?;
    let obj = record(&value).ok_or_else(|| error("invalid_envelope", None))?;
    for field in ["msg_id", "corr_id"] {
        if !text(obj.get(field)) {
            return Err(invalid(field));
        }
    }
    if !obj
        .get("t_mono_ns")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(invalid("t_mono_ns"));
    }
    if !number(obj.get("t_wall_ms")) || obj["t_wall_ms"].as_f64().unwrap() < 0.0 {
        return Err(invalid("t_wall_ms"));
    }
    let kind = obj
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| error("unknown_kind", Some("kind")))?;
    if !["CMD", "ACK", "EVT", "TELEM", "QRY", "RPY", "ERR"].contains(&kind) {
        return Err(error("unknown_kind", Some("kind")));
    }
    let topic = obj
        .get("topic")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("topic"))?;
    if !allowed_kind(topic, kind) {
        return Err(invalid("kind"));
    }
    if !obj
        .get("seq")
        .and_then(Value::as_u64)
        .is_some_and(|n| n <= u32::MAX as u64)
    {
        return Err(invalid("seq"));
    }
    let payload = obj
        .get("payload")
        .and_then(record)
        .ok_or_else(|| invalid("payload"))?;
    if serde_json::to_vec(payload).is_ok_and(|bytes| bytes.len() > MAX_BYTES) {
        return Err(error("oversized_payload", Some("payload")));
    }
    if kind == "ACK" {
        let valid = if topic == "sys.receipt" {
            payload
                .get("msg_ids")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().all(|id| text(Some(id))))
        } else {
            payload.get("accepted").and_then(Value::as_bool).is_some()
        };
        if !valid {
            return Err(invalid("payload"));
        }
    } else if !payload_valid(topic, payload) {
        return Err(error("malformed", Some("payload")));
    }
    if kind == "CMD" {
        if !text(obj.get("idem_key")) {
            return Err(invalid("idem_key"));
        }
    } else if obj.contains_key("idem_key") {
        return Err(invalid("idem_key"));
    }
    if let Some(expires) = obj.get("expires_at") {
        if !number(Some(expires)) || kind != "CMD" {
            return Err(invalid("expires_at"));
        }
        if expires.as_f64().unwrap() < obj["t_wall_ms"].as_f64().unwrap() {
            return Err(error("expired", Some("expires_at")));
        }
    }
    Ok(value)
}

/// Parse one phone-to-robot frame; invalid frames are dropped without a reply.
pub fn parse_teleop_frame(raw: &str) -> Option<Value> {
    if raw.encode_utf16().count() > 256 {
        return None;
    }
    let value: Value = serde_json::from_str(raw).ok()?;
    let obj = value.as_object()?;
    let mut frame = Map::new();
    let kind = obj.get("t")?.as_str()?;
    frame.insert("t".into(), Value::String(kind.into()));
    match kind {
        "hello" if text(obj.get("token")) && number(obj.get("protocol_version")) => {
            frame.insert("token".into(), obj["token"].clone());
            frame.insert("protocol_version".into(), obj["protocol_version"].clone());
        }
        "welcome"
            if obj
                .get("deadman_ms")
                .and_then(Value::as_f64)
                .is_some_and(|n| n > 0.0) =>
        {
            frame.insert("deadman_ms".into(), obj["deadman_ms"].clone());
        }
        "drive" if positive_int(obj.get("seq")) && number(obj.get("v")) && number(obj.get("w")) => {
            for key in ["seq", "v", "w"] {
                frame.insert(key.into(), obj[key].clone());
            }
        }
        "status"
            if obj.get("seq").and_then(Value::as_u64).is_some()
                && obj.get("deadman").and_then(Value::as_bool).is_some() =>
        {
            for key in ["seq", "deadman"] {
                frame.insert(key.into(), obj[key].clone());
            }
        }
        _ => return None,
    }
    Some(Value::Object(frame))
}
