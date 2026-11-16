use std::time::Duration;

pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
pub const REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
pub const SCOPES: &str = "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
pub const OAUTH_TIMEOUT: Duration = Duration::from_secs(60);
pub const USER_AGENT: &str = "claude-cli/2.1.280";
pub const ANTHROPIC_BETA: &str = "claude-code-20250219,oauth-2025-04-20";
