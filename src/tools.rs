use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};
use twilight_http::{request::channel::reaction::RequestReactionType, Client, Response};
use twilight_model::{channel::message::{AllowedMentions, MentionType}, channel::ChannelType, guild::audit_log::AuditLogEventType, http::attachment::Attachment, id::Id, util::Timestamp};

type R = Result<Value, String>;

/// Tool parameter: (name, JSON type, required, description).
type P = (&'static str, &'static str, bool, &'static str);
const G: P = ("guild_id", "string", false, "Guild (server) ID; defaults to default_guild_id from config");
const C: P = ("channel_id", "string", true, "Channel ID");
const M: P = ("message_id", "string", true, "Message ID");
const U: P = ("user_id", "string", true, "User ID");
const RO: P = ("role_id", "string", true, "Role ID");
const EMOJI: P = ("emoji", "string", true, "Unicode emoji, or custom emoji as name:id or <:name:id>");

const TOOLS: &[(&str, &str, &[P])] = &[
    ("list_guilds", "List guilds (servers) the bot is in", &[]),
    ("get_guild", "Get guild details", &[G]),
    ("list_channels", "List channels of a guild", &[G]),
    ("list_roles", "List roles of a guild", &[G]),
    ("list_members", "List guild members (needs Server Members intent enabled for the bot); members are ordered by user ID, page with after", &[G, ("limit", "integer", false, "1-1000, default 100"), ("after", "string", false, "Only members with a user ID greater than this")]),
    ("read_messages", "Read messages of a channel (newest first); page with before, poll for new ones with after", &[C, ("limit", "integer", false, "1-100, default 20"), ("before", "string", false, "Only messages older than this message ID"), ("after", "string", false, "Only messages newer than this message ID"), ("around", "string", false, "Messages around this message ID")]),
    ("get_message", "Get a single message", &[C, M]),
    ("get_mentions", "Find recent messages that mention the bot or reply to it, across the guild's text channels (newest first)", &[G, ("after", "string", false, "Only messages newer than this message ID (pass the newest id from the last call to get only new ones)"), ("per_channel", "integer", false, "Messages scanned per channel, 1-100, default 50"), ("channel_ids", "array", false, "Only scan these channel/thread IDs"), ("everyone", "boolean", false, "Also include @everyone/@here and mentions of the bot's roles")]),
    ("send_message", "Send a message to a channel, or a DM to a user; supports replies and file/image attachments", &[("channel_id", "string", false, "Channel ID (required unless user_id is set)"), ("user_id", "string", false, "Send as a DM to this user instead"), ("content", "string", false, "Message text (required unless files are attached)"), ("reply_to", "string", false, "Message ID to reply to"), ("ping", "boolean", false, "Ping the author of reply_to, default true"), ("file", "string", false, "Local file path to attach"), ("files", "array", false, "Local file paths to attach (max 10)")]),
    ("edit_message", "Edit a message sent by the bot", &[C, M, ("content", "string", true, "New text")]),
    ("delete_message", "Delete a message", &[C, M]),
    ("add_reaction", "React to a message", &[C, M, EMOJI]),
    ("remove_reaction", "Remove the bot's reaction from a message", &[C, M, EMOJI]),
    ("get_reactions", "List users who reacted to a message with an emoji", &[C, M, EMOJI, ("limit", "integer", false, "1-100, default 25")]),
    ("pin_message", "Pin a message", &[C, M]),
    ("create_channel", "Create a guild channel", &[G, ("name", "string", true, "Channel name"), ("type", "string", false, "text (default), voice, category, announcement, forum, stage"), ("topic", "string", false, "Topic"), ("parent_id", "string", false, "Category ID")]),
    ("edit_channel", "Edit a channel's name and/or topic", &[C, ("name", "string", false, "New name"), ("topic", "string", false, "New topic")]),
    ("delete_channel", "Delete a channel", &[C]),
    ("add_role", "Give a member a role", &[G, U, RO]),
    ("remove_role", "Remove a role from a member", &[G, U, RO]),
    ("kick_member", "Kick a member", &[G, U]),
    ("ban_member", "Ban a user", &[G, U, ("delete_message_seconds", "integer", false, "Delete their messages from the last N seconds (max 604800)")]),
    ("unban_member", "Unban a user", &[G, U]),
    ("get_user", "Get a user's profile, plus their member info in the guild if they are in it", &[("user_id", "string", false, "User ID; omit for the bot itself"), G]),
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
                let mut prop = json!({ "type": ty, "description": d });
                if *ty == "array" {
                    prop["items"] = json!({ "type": "string" });
                }
                props.insert(p.to_string(), prop);
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

/// True if the argument is present as a non-empty string or a number.
fn has(a: &Value, k: &str) -> bool {
    str_arg(a, k).is_some() || a.get(k).is_some_and(Value::is_number)
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
    match v {
        Value::Array(a) => a.iter().map(simplify_message).collect(),
        m => simplify_message(&m),
    }
}

fn simplify_message(m: &Value) -> Value {
    let list = |k: &str, f: &dyn Fn(&Value) -> Value| -> Vec<Value> { m[k].as_array().map(|x| x.iter().map(f).collect()).unwrap_or_default() };
    let mut out = json!({
        "id": m["id"],
        "channel_id": m["channel_id"],
        "author": m["author"]["username"],
        "author_id": m["author"]["id"],
        "content": m["content"],
        "timestamp": m["timestamp"],
        "attachments": list("attachments", &|f| f["url"].clone()),
        "embeds": list("embeds", &embed_text),
    });
    let o = out.as_object_mut().unwrap();
    if let Some(s) = m["author"]["global_name"].as_str() {
        o.insert("author_name".into(), s.into());
    }
    if m["author"]["bot"] == true {
        o.insert("author_bot".into(), true.into());
    }
    if let Some(r) = m.get("message_reference").and_then(|r| r.get("message_id")) {
        o.insert("reply_to".into(), r.clone());
        if let Some(a) = m["referenced_message"]["author"]["id"].as_str() {
            o.insert("reply_to_author_id".into(), a.into());
        }
    }
    let mentions = list("mentions", &|u| json!({ "id": u["id"], "username": u["username"] }));
    if !mentions.is_empty() {
        o.insert("mentions".into(), mentions.into());
    }
    if m["mention_roles"].as_array().is_some_and(|r| !r.is_empty()) {
        o.insert("mention_roles".into(), m["mention_roles"].clone());
    }
    if m["mention_everyone"] == true {
        o.insert("mention_everyone".into(), true.into());
    }
    let reactions = list("reactions", &|r| {
        let e = &r["emoji"];
        let name = match e["id"].as_str() {
            Some(id) => format!("{}:{id}", e["name"].as_str().unwrap_or("")),
            None => e["name"].as_str().unwrap_or("").to_string(),
        };
        json!({ "emoji": name, "count": r["count"], "me": r["me"] })
    });
    if !reactions.is_empty() {
        o.insert("reactions".into(), reactions.into());
    }
    let mut parts = vec![];
    for c in m["components"].as_array().into_iter().flatten() {
        component_text(c, &mut parts);
    }
    if !parts.is_empty() {
        o.insert("components".into(), Value::String(parts.join("\n")));
    }
    out
}

/// Parse "😀", "name:id" or "<:name:id>" / "<a:name:id>" into a reaction emoji.
fn emoji(s: &str) -> Result<RequestReactionType<'_>, String> {
    let t = s.trim_start_matches('<').trim_end_matches('>');
    let t = t.strip_prefix("a:").unwrap_or(t).trim_start_matches(':');
    match t.rsplit_once(':') {
        Some((name, id)) if !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit()) => Ok(RequestReactionType::Custom {
            id: id.parse().ok().and_then(Id::new_checked).ok_or("invalid emoji id")?,
            name: Some(name).filter(|n| !n.is_empty()),
        }),
        _ => Ok(RequestReactionType::Unicode { name: s }),
    }
}

/// CDN URL for a user avatar or banner hash (animated hashes start with a_).
fn cdn(kind: &str, id: &Value, hash: &Value) -> Value {
    match (id.as_str(), hash.as_str()) {
        (Some(id), Some(h)) => format!("https://cdn.discordapp.com/{kind}/{id}/{h}.{}?size=1024", if h.starts_with("a_") { "gif" } else { "png" }).into(),
        _ => Value::Null,
    }
}

/// Unix seconds encoded in a snowflake ID.
fn snowflake_secs(id: &Value) -> Option<u64> {
    id.as_str().and_then(|i| i.parse::<u64>().ok()).map(|i| ((i >> 22) + 1_420_070_400_000) / 1000)
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
                    let ts = snowflake_secs(&e["id"]);
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
    for v in [&e["image"]["url"], &e["thumbnail"]["url"]] {
        if let Some(s) = v.as_str() {
            parts.push(s.to_string());
        }
    }
    Value::String(parts.join("
"))
}

/// Collect text and media URLs from nested message components (Components V2 layouts).
fn component_text(c: &Value, parts: &mut Vec<String>) {
    for v in [&c["content"], &c["media"]["url"], &c["file"]["url"]] {
        if let Some(s) = v.as_str() {
            parts.push(s.to_string());
        }
    }
    for key in ["components", "items"] {
        for x in c[key].as_array().into_iter().flatten() {
            component_text(x, parts);
        }
    }
    if c["accessory"].is_object() {
        component_text(&c["accessory"], parts);
    }
}

fn ok(_: Value) -> Value {
    json!({ "ok": true })
}

pub struct Ctx {
    http: Client,
    default_guild: Option<u64>,
    me: OnceLock<String>,
}

impl Ctx {
    pub fn new(cfg: crate::config::Config) -> Self {
        Self { http: Client::new(cfg.token), default_guild: cfg.default_guild_id, me: OnceLock::new() }
    }

    fn guild<T>(&self, a: &Value) -> Result<Id<T>, String> {
        num_arg(a, "guild_id")
            .or(self.default_guild)
            .and_then(Id::new_checked)
            .ok_or_else(|| "guild_id missing and no default_guild_id configured".into())
    }

    /// The bot's own user ID, fetched once.
    async fn me(&self) -> Result<&str, String> {
        if self.me.get().is_none() {
            let u = body(self.http.current_user().await).await?;
            let _ = self.me.set(u["id"].as_str().ok_or("no user id")?.to_string());
        }
        Ok(self.me.get().unwrap())
    }

    /// Scan channels for messages that mention or reply to the bot. No gateway, so this polls.
    async fn mentions(&self, a: &Value) -> R {
        let h = &self.http;
        let me = self.me().await?;
        let everyone = a["everyone"] == true;
        let per = num_arg(a, "per_channel").unwrap_or(50).clamp(1, 100) as u16;
        let after = if has(a, "after") { Some(id(a, "after")?) } else { None };
        let channels: Vec<u64> = match a["channel_ids"].as_array() {
            Some(ids) => ids.iter().filter_map(|v| v.as_str().and_then(|s| s.parse().ok()).or(v.as_u64())).collect(),
            None => body(h.guild_channels(self.guild(a)?).await)
                .await?
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| matches!(c["type"].as_u64(), Some(0 | 5)))
                .filter_map(|c| c["id"].as_str()?.parse().ok())
                .collect(),
        };
        let my_roles: Vec<Value> = match (everyone, self.guild(a), me.parse().ok().and_then(Id::new_checked)) {
            (true, Ok(g), Some(uid)) => body(h.guild_member(g, uid).await).await.ok().and_then(|m| m["roles"].as_array().cloned()).unwrap_or_default(),
            _ => vec![],
        };
        let mut found = vec![];
        for cid in channels.into_iter().filter_map(Id::new_checked) {
            let req = h.channel_messages(cid);
            let res = match after {
                Some(m) => req.after(m).limit(per).await,
                None => req.limit(per).await,
            };
            // Channels the bot cannot read are skipped.
            let Ok(Value::Array(ms)) = body(res).await else { continue };
            for m in ms {
                let hit = m["author"]["id"] != me
                    && (m["mentions"].as_array().into_iter().flatten().any(|u| u["id"] == me)
                        || m["referenced_message"]["author"]["id"] == me
                        || (everyone && (m["mention_everyone"] == true || m["mention_roles"].as_array().into_iter().flatten().any(|r| my_roles.contains(r)))));
                if hit {
                    found.push(simplify_message(&m));
                }
            }
        }
        found.sort_by_key(|v| std::cmp::Reverse(v["id"].as_str().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0)));
        Ok(found.into())
    }

    async fn user(&self, a: &Value) -> R {
        let h = &self.http;
        let u = if has(a, "user_id") { body(h.user(id(a, "user_id")?).await).await? } else { body(h.current_user().await).await? };
        let mut out = json!({
            "id": u["id"],
            "username": u["username"],
            "global_name": u["global_name"],
            "bot": u["bot"],
            "avatar": cdn("avatars", &u["id"], &u["avatar"]),
            "banner": cdn("banners", &u["id"], &u["banner"]),
            "accent_color": u["accent_color"],
            "public_flags": u["public_flags"],
            "created_at": snowflake_secs(&u["id"]),
        });
        // Member info is optional: the user may not be in the guild.
        if let (Ok(g), Ok(uid)) = (self.guild(a), id(&u, "id")) {
            if let Ok(m) = body(h.guild_member(g, uid).await).await {
                let member = json!({
                    "nick": m["nick"],
                    "roles": m["roles"],
                    "joined_at": m["joined_at"],
                    "premium_since": m["premium_since"],
                    "timed_out_until": m["communication_disabled_until"],
                    "guild_avatar": m["avatar"].as_str().map(|hash| format!("https://cdn.discordapp.com/guilds/{g}/users/{uid}/avatars/{hash}.png?size=1024")),
                });
                out["member"] = pick(member, &["nick", "roles", "joined_at", "premium_since", "timed_out_until", "guild_avatar"]);
            }
        }
        Ok(pick(out, &["id", "username", "global_name", "bot", "avatar", "banner", "accent_color", "public_flags", "created_at", "member"]))
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
                let req = h.guild_members(self.guild(a)?).limit(limit);
                let v = if has(a, "after") { body(req.after(id(a, "after")?).await).await? } else { body(req.await).await? };
                let Value::Array(ms) = v else { return Ok(v) };
                ms.into_iter()
                    .map(|m| json!({ "id": m["user"]["id"], "username": m["user"]["username"], "nick": m["nick"], "roles": m["roles"], "bot": m["user"]["bot"], "joined_at": m["joined_at"] }))
                    .collect()
            }
            "read_messages" => {
                let limit = num_arg(a, "limit").unwrap_or(20).clamp(1, 100) as u16;
                let req = h.channel_messages(id(a, "channel_id")?);
                let res = if has(a, "before") {
                    req.before(id(a, "before")?).limit(limit).await
                } else if has(a, "after") {
                    req.after(id(a, "after")?).limit(limit).await
                } else if has(a, "around") {
                    req.around(id(a, "around")?).limit(limit).await
                } else {
                    req.limit(limit).await
                };
                simplify_messages(body(res).await?)
            }
            "get_message" => simplify_messages(body(h.message(id(a, "channel_id")?, id(a, "message_id")?).await).await?),
            "get_mentions" => self.mentions(a).await?,
            "get_user" => self.user(a).await?,
            "send_message" => {
                let mut paths: Vec<&str> = a["files"].as_array().map(|x| x.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                paths.extend(str_arg(a, "file"));
                if paths.len() > 10 {
                    return Err("at most 10 files".into());
                }
                let mut files = Vec::new();
                for (i, path) in paths.into_iter().enumerate() {
                    let name = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
                    files.push(Attachment::from_bytes(name, std::fs::read(path).map_err(|e| format!("{path}: {e}"))?, i as u64));
                }
                let channel = if has(a, "channel_id") {
                    id(a, "channel_id")?
                } else if has(a, "user_id") {
                    id(&body(h.create_private_channel(id(a, "user_id")?).await).await?, "id")?
                } else {
                    return Err("missing argument: channel_id or user_id".into());
                };
                let mut req = h.create_message(channel).attachments(&files);
                if files.is_empty() || str_arg(a, "content").is_some() {
                    req = req.content(need_str(a, "content")?);
                }
                if has(a, "reply_to") {
                    req = req.reply(id(a, "reply_to")?);
                }
                let no_ping = AllowedMentions { parse: vec![MentionType::Users, MentionType::Roles, MentionType::Everyone], ..Default::default() };
                if a["ping"] == false {
                    req = req.allowed_mentions(Some(&no_ping));
                }
                pick(body(req.await).await?, &["id", "channel_id"])
            }
            "edit_message" => pick(
                body(h.update_message(id(a, "channel_id")?, id(a, "message_id")?).content(Some(need_str(a, "content")?)).await).await?,
                &["id", "content"],
            ),
            "delete_message" => ok(body(h.delete_message(id(a, "channel_id")?, id(a, "message_id")?).await).await?),
            "add_reaction" => {
                let e = emoji(need_str(a, "emoji")?)?;
                ok(body(h.create_reaction(id(a, "channel_id")?, id(a, "message_id")?, &e).await).await?)
            }
            "remove_reaction" => {
                let e = emoji(need_str(a, "emoji")?)?;
                ok(body(h.delete_current_user_reaction(id(a, "channel_id")?, id(a, "message_id")?, &e).await).await?)
            }
            "get_reactions" => {
                let e = emoji(need_str(a, "emoji")?)?;
                let limit = num_arg(a, "limit").unwrap_or(25).clamp(1, 100) as u16;
                pick(body(h.reactions(id(a, "channel_id")?, id(a, "message_id")?, &e).limit(limit).await).await?, &["id", "username", "global_name", "bot"])
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
