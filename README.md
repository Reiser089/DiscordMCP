# DiscordMCP

A lightweight [MCP](https://modelcontextprotocol.io) server, written in Rust with [Twilight](https://twilight.rs), for managing your Discord server through a bot.

- REST only: no gateway, cache or websocket. It sits idle until a tool is called.
- 4 dependencies: `twilight-http`, `twilight-model`, `tokio`, `serde_json`. JSON-RPC and the config parser are written by hand.
- About 1.3 MB release binary.

## Setup

1. **Create a bot.** Go to https://discord.com/developers/applications, then New Application, then Bot, and copy the token.
   - To use `list_members`, turn on **Server Members Intent**.
   - To read message text, turn on **Message Content Intent**.
2. **Invite the bot.** Go to OAuth2, then URL Generator, and pick scope `bot`. Grant the permissions you want, for example Administrator.
3. **Configure.** Copy `config.example.toml` to `config.toml` and put your token in it. You can set `default_guild_id` so you don't have to name the server each time.
   The server looks for `config.toml` in this order:
   1. `$DISCORD_MCP_CONFIG`
   2. next to the exe
   3. the project root
4. **Build.** Run `cargo build --release`.
5. **Register** it as a stdio MCP server in your MCP client:
   ```json
   { "mcpServers": { "discord": { "command": "C:\\path\\to\\DiscordMCP\\target\\release\\discord-mcp.exe" } } }
   ```

## Tools

| Area | Tools |
|---|---|
| Info | `list_guilds`, `get_guild`, `list_channels`, `list_roles`, `list_members`, `get_user` |
| Messages | `read_messages`, `get_message`, `get_mentions`, `transcribe_audio` (speech-to-text of voice messages, see below), `send_message` (replies, DMs, files), `edit_message`, `delete_message`, `pin_message` |
| Reactions | `add_reaction`, `remove_reaction`, `get_reactions` |
| Channels | `create_channel`, `edit_channel`, `delete_channel` |
| Moderation | `add_role`, `remove_role`, `kick_member`, `ban_member`, `unban_member`, `timeout_member` |

## Voice messages

`transcribe_audio` takes a `channel_id` and `message_id`, picks the first audio attachment (voice messages included) and runs the program named by `transcribe_cmd` in `config.toml` with the attachment URL as its only argument. Whatever the program prints is returned, as JSON if it parses.

`scripts/transcribe.py` is a ready-made helper built on [faster-whisper](https://github.com/SYSTRAN/faster-whisper): `pip install faster-whisper`, then point `transcribe_cmd` at a small wrapper script that runs `python scripts/transcribe.py <url>`. The model (`WHISPER_MODEL`, default `base`) is downloaded on first use into `WHISPER_DIR`.
