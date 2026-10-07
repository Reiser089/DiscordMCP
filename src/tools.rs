use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};
use twilight_http::{request::channel::reaction::RequestReactionType, Client, Response};
use twilight_model::{channel::ChannelType, guild::audit_log::AuditLogEventType, id::Id, util::Timestamp};

type R = Result<Value, String>;

/// Tool parameter: (name, JSON type, required, description).
type P = (&'static str, &'static str, bool, &'static str);
const G: P = ("guild_id", "string", false, "Guild (server) ID; defaults to default_guild_id from config");
const C: P = ("channel_id", "string", true, "Channel ID");
const M: P = ("message_id", "string", true, "Message ID");
const U: P = ("user_id", "string", true, "User ID");
const RO: P = ("role_id", "string", true, "Role ID");

const TOOLS: &[(&str, &str, &[P])] = &[
    ("list_guilds", "List guilds (servers) the bot is in", &[]),
    ("get_guild", "Get guild details", &[G]),
    ("list_channels", "List channels of a guild", &[G]),
    ("list_roles", "List roles of a guild", &[G]),
    ("list_members", "List guild members (needs Server Members intent enabled for the bot)", &[G, ("limit", "integer", false, "1-1000, default 100")]),
    ("read_messages", "Read messages of a channel (newest first); page with before", &[C, ("limit", "integer", false, "1-100, default 20"), ("before", "string", false, "Only messages older than this message ID")]),
    ("send_message", "Send a message to a channel", &[C, ("content", "string", true, "Message text"), ("reply_to", "string", false, "Message ID to reply to")]),
    ("edit_message", "Edit a message sent by the bot", &[C, M, ("content", "string", true, "New text")]),
    ("delete_message", "Delete a message", &[C, M]),
    ("add_reaction", "React to a message with a unicode emoji", &[C, M, ("emoji", "string", true, "Unicode emoji character")]),
    ("pin_message", "Pin a message", &[C, M]),
    ("create_channel", "Create a guild channel", &[G, ("name", "string", true, "Channel name"), ("type", "string", false, "text (default), voice, category, announcement, forum, stage"), ("topic", "string", false, "Topic"), ("parent_id", "string", false, "Category ID")]),
    ("edit_channel", "Edit a channel's name and/or topic", &[C, ("name", "string", false, "New name"), ("topic", "string", false, "New topic")]),
    ("delete_channel", "Delete a channel", &[C]),
    ("add_role", "Give a member a role", &[G, U, RO]),
    ("remove_role", "Remove a role from a member", &[G, U, RO]),
    ("kick_member", "Kick a member", &[G, U]),
    ("ban_member", "Ban a user", &[G, U, ("delete_message_seconds", "integer", false, "Delete their messages from the last N seconds (max 604800)")]),
    ("unban_member", "Unban a user", &[G, U]),
    ("get_audit_log", "Read the guild audit log (newest first, max 100 per call; page with before)", &[G, ("limit", "integer", false, "1-100, default 100"), ("before", "string", false, "Only entries older than this entry ID"), ("action_type", "integer", false, "Filter: 20 kick, 22 ban, 23 unban, 24 member update/timeout, 25 role update, 72 message delete"), ("user_id", "string", false, "Only actions by this user")]),
    ("timeout_member","Time out a member; minutes=0 removes the timeout", &[G, U, ("minutes", "integer", true, "Duration in minutes (max 40320)")]),
];

pub fn list() -> Value {
    TOOLS
        .iter()
        .map(|(name, desc, params)| {
            let mut props = Map::new();
            let mut required = vec![];
            for (p, ty, req, d) in params.iter() {
                props.insert(p.to_string(), json!({ "type": ty, "description": d }));
                if *req {
                    required.push(*p);
                }
            }
            json!({
                "name": name,
                "description": desc,
                "inputSchema": { "type": "object", "properties": props, "required": required }
            })
        })
        .collect()
}

fn str_arg<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Numbers may arrive as JSON numbers or strings (snowflakes exceed JS precision).
fn num_arg(a: &Value, k: &str) -> Option<u64> {
    match a.get(k)? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn need_str<'a>(a: &'a Value, k: &str) -> Result<&'a str, String> {
    str_arg(a, k).ok_or_else(|| format!("missing argument: {k}"))
}

fn id<T>(a: &Value, k: &str) -> Result<Id<T>, String> {
    num_arg(a, k).and_then(Id::new_checked).ok_or_else(|| format!("missing or invalid id: {k}"))
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Read the raw JSON body; works for single models, lists and empty (204) responses.
async fn body<T>(res: Result<Response<T>, twilight_http::Error>) -> R {
    let bytes = res.map_err(err)?.bytes().await.map_err(err)?;
    if bytes.is_empty() {
        return Ok(json!({ "ok": true }));
    }
    serde_json::from_slice(&bytes).map_err(err)
}

/// Keep only the listed non-null fields of an object (or of each object in an array).
fn pick(v: Value, keys: &[&str]) -> Value {
    match v {
        Value::Array(a) => a.into_iter().map(|x| pick(x, keys)).collect(),
        Value::Object(o) => o.into_iter().filter(|(k, v)| keys.contains(&k.as_str()) && !v.is_null()).collect(),
        x => x,
    }
}

fn simplify_messages(v: Value) -> Value {
    let Value::Array(a) = v else { return v };
    a.into_iter()
        .map(|m| {
            let files: Vec<Value> = m["attachments"].as_array().map(|x| x.iter().map(|f| f["url"].clone()).collect()).unwrap_or_default();
            let mut out = json!({
                "id": m["id"],
                "author": m["author"]["username"],
                "author_id": m["author"]["id"],
                "content": m["content"],
                "timestamp": m["timestamp"],
                "attachments": files,
                "embeds": m["embeds"].as_array().map(|x| x.iter().map(embed_text).collect::<Vec<_>>()).unwrap_or_default(),
            });
            if let Some(msg_ref) = m.get("message_reference").and_then(|r| r.get("message_id")) {
                out.as_object_mut().unwrap().insert("reply_to".to_string(), msg_ref.clone());
            }
            out
        })
        .collect()
}

/// Flatten audit log entries; resolves user names and adds unix time from the snowflake.
fn simplify_audit_log(v: Value) -> Value {
    let names: Map<String, Value> = v["users"]
        .as_array()
        .map(|us| us.iter().map(|u| (u["id"].as_str().unwrap_or("").to_string(), u["username"].clone())).collect())
        .unwrap_or_default();
    let name = |id: &Value| id.as_str().and_then(|i| names.get(i)).cloned().unwrap_or(Value::Null);
    v["audit_log_entries"]
        .as_array()
        .map(|es| {
            es.iter()
                .map(|e| {
                    let ts = e["id"].as_str().and_then(|i| i.parse::<u64>().ok()).map(|i| ((i >> 22) + 1_420_070_400_000) / 1000);
                    let changes: Vec<Value> = e["changes"].as_array().map(|c| c.iter().map(|x| x["key"].clone()).collect()).unwrap_or_default();
                    json!({
                        "id": e["id"],
                        "time": ts,
                        "action": e["action_type"],
                        "by": name(&e["user_id"]),
                        "by_id": e["user_id"],
                        "target": name(&e["target_id"]),
                        "target_id": e["target_id"],
                        "reason": e["reason"],
                        "changes": changes,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Flatten an embed (author, title, description, fields, footer) into one text block.
fn embed_text(e: &Value) -> Value {
    let mut parts = vec![];
    for v in [&e["author"]["name"], &e["title"], &e["description"]] {
        if let Some(s) = v.as_str() {
            parts.push(s.to_string());
        }
    }
    for f in e["fields"].as_array().into_iter().flatten() {
        parts.push(format!("{}: {}", f["name"].as_str().unwrap_or(""), f["value"].as_str().unwrap_or("")));
    }
    if let Some(s) = e["footer"]["text"].as_str() {
        parts.push(s.to_string());
    }
    Value::String(parts.join("
"))
}

fn ok(_: Value) -> Value {
    json!({ "ok": true })
}

pub struct Ctx {
    http: Client,
    default_guild: Option<u64>,
}

impl Ctx {
    pub fn new(cfg: crate::config::Config) -> Self {
        Self { http: Client::new(cfg.token), default_guild: cfg.default_guild_id }
    }

    fn guild<T>(&self, a: &Value) -> Result<Id<T>, String> {
        num_arg(a, "guild_id")
            .or(self.default_guild)
            .and_then(Id::new_checked)
            .ok_or_else(|| "guild_id missing and no default_guild_id configured".into())
    }

    pub async fn call(&self, name: &str, a: &Value) -> R {
        let h = &self.http;
        Ok(match name {
            "list_guilds" => pick(body(h.current_user_guilds().await).await?, &["id", "name", "owner", "permissions"]),
            "get_guild" => pick(
                body(h.guild(self.guild(a)?).with_counts(true).await).await?,
                &["id", "name", "description", "owner_id", "approximate_member_count", "approximate_presence_count", "premium_tier"],
            ),
            "list_channels" => pick(body(h.guild_channels(self.guild(a)?).await).await?, &["id", "name", "type", "parent_id", "position", "topic"]),
            "list_roles" => pick(body(h.roles(self.guild(a)?).await).await?, &["id", "name", "color", "position", "permissions", "managed"]),
            "list_members" => {
                let limit = num_arg(a, "limit").unwrap_or(100).clamp(1, 1000) as u16;
                let v = body(h.guild_members(self.guild(a)?).limit(limit).await).await?;
                let Value::Array(ms) = v else { return Ok(v) };
                ms.into_iter()
                    .map(|m| json!({ "id": m["user"]["id"], "username": m["user"]["username"], "nick": m["nick"], "roles": m["roles"], "bot": m["user"]["bot"] }))
                    .collect()
            }
            "read_messages" => {
                let limit = num_arg(a, "limit").unwrap_or(20).clamp(1, 100) as u16;
                let req = h.channel_messages(id(a, "channel_id")?);
                let res = if str_arg(a, "before").is_some() || a.get("before").is_some_and(Value::is_number) {
                    req.before(id(a, "before")?).limit(limit).await
                } else {
                    req.limit(limit).await
                };
                simplify_messages(body(res).await?)
            }
            "send_message" => {
                let mut req = h.create_message(id(a, "channel_id")?).content(need_str(a, "content")?);
                if str_arg(a, "reply_to").is_some() {
                    req = req.reply(id(a, "reply_to")?);
                }
                pick(body(req.await).await?, &["id", "channel_id"])
            }
            "edit_message" => pick(
                body(h.update_message(id(a, "channel_id")?, id(a, "message_id")?).content(Some(need_str(a, "content")?)).await).await?,
                &["id", "content"],
            ),
            "delete_message" => ok(body(h.delete_message(id(a, "channel_id")?, id(a, "message_id")?).await).await?),
            "add_reaction" => {
                let emoji = RequestReactionType::Unicode { name: need_str(a, "emoji")? };
                ok(body(h.create_reaction(id(a, "channel_id")?, id(a, "message_id")?, &emoji).await).await?)
            }
            "pin_message" => ok(body(h.create_pin(id(a, "channel_id")?, id(a, "message_id")?).await).await?),
            "create_channel" => {
                let kind = match str_arg(a, "type").unwrap_or("text") {
                    "text" => ChannelType::GuildText,
                    "voice" => ChannelType::GuildVoice,
                    "category" => ChannelType::GuildCategory,
                    "announcement" => ChannelType::GuildAnnouncement,
                    "forum" => ChannelType::GuildForum,
                    "stage" => ChannelType::GuildStageVoice,
                    t => return Err(format!("unknown channel type: {t}")),
                };
                let mut req = h.create_guild_channel(self.guild(a)?, need_str(a, "name")?).kind(kind);
                if let Some(t) = str_arg(a, "topic") {
                    req = req.topic(t);
                }
                if str_arg(a, "parent_id").is_some() {
                    req = req.parent_id(id(a, "parent_id")?);
                }
                pick(body(req.await).await?, &["id", "name", "type"])
            }
            "edit_channel" => {
                let mut req = h.update_channel(id(a, "channel_id")?);
                if let Some(n) = str_arg(a, "name") {
                    req = req.name(n);
                }
                if let Some(t) = str_arg(a, "topic") {
                    req = req.topic(t);
                }
                pick(body(req.await).await?, &["id", "name", "topic"])
            }
            "delete_channel" => pick(body(h.delete_channel(id(a, "channel_id")?).await).await?, &["id", "name"]),
            "add_role" => ok(body(h.add_guild_member_role(self.guild(a)?, id(a, "user_id")?, id(a, "role_id")?).await).await?),
            "remove_role" => ok(body(h.remove_guild_member_role(self.guild(a)?, id(a, "user_id")?, id(a, "role_id")?).await).await?),
            "kick_member" => ok(body(h.remove_guild_member(self.guild(a)?, id(a, "user_id")?).await).await?),
            "ban_member" => {
                let mut req = h.create_ban(self.guild(a)?, id(a, "user_id")?);
                if let Some(s) = num_arg(a, "delete_message_seconds") {
                    req = req.delete_message_seconds(s.min(604_800) as u32);
                }
                ok(body(req.await).await?)
            }
            "unban_member" => ok(body(h.delete_ban(self.guild(a)?, id(a, "user_id")?).await).await?),
            "get_audit_log" => {
                let limit = num_arg(a, "limit").unwrap_or(100).clamp(1, 100) as u16;
                let mut req = h.audit_log(self.guild(a)?).limit(limit);
                if let Some(b) = num_arg(a, "before") {
                    req = req.before(b);
                }
                if let Some(t) = num_arg(a, "action_type") {
                    req = req.action_type(AuditLogEventType::from(t as u16));
                }
                if str_arg(a, "user_id").is_some() || a.get("user_id").is_some_and(Value::is_number) {
                    req = req.user_id(id(a, "user_id")?);
                }
                simplify_audit_log(body(req.await).await?)
            }
            "timeout_member" => {
                let minutes = num_arg(a, "minutes").ok_or("missing argument: minutes")?.min(40_320);
                let until = if minutes == 0 {
                    None
                } else {
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(err)?.as_secs();
                    Some(Timestamp::from_secs((now + minutes * 60) as i64).map_err(err)?)
                };
                ok(body(h.update_guild_member(self.guild(a)?, id(a, "user_id")?).communication_disabled_until(until).await).await?)
            }
            _ => return Err(format!("unknown tool: {name}")),
        })
    }
}
