use std::{env, fs, path::PathBuf};

pub struct Config {
    pub token: String,
    pub default_guild_id: Option<u64>,
}

/// Path: $DISCORD_MCP_CONFIG, else config.toml next to the executable,
/// else config.toml in the project directory the binary was built from.
fn path() -> PathBuf {
    if let Ok(p) = env::var("DISCORD_MCP_CONFIG") {
        return p.into();
    }
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config.toml");
    env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("config.toml")))
        .filter(|p| p.exists())
        .unwrap_or(project)
}

/// Minimal `key = "value"` parser; ignores blank lines and `#` comments.
fn get(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (k, v) = line.trim().split_once('=')?;
        if k.trim() != key {
            return None;
        }
        let v = v.trim().trim_matches('"').trim();
        (!v.is_empty()).then(|| v.to_string())
    })
}

pub fn load() -> Result<Config, String> {
    let p = path();
    let text = fs::read_to_string(&p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
    let token = get(&text, "token")
        .filter(|t| t != "YOUR_BOT_TOKEN")
        .ok_or_else(|| format!("no bot token set in {}", p.display()))?;
    let default_guild_id = get(&text, "default_guild_id").and_then(|g| g.parse().ok());
    Ok(Config { token, default_guild_id })
}
