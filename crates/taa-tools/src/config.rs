use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub bind: String,
    pub agent_access: AgentAccess,
    pub sessions: Sessions,
    pub screenshots: Screenshots,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentAccess {
    pub computer_vision: bool,
    pub computer_input: bool,
    pub forward_mcp_tools: bool,
    pub screen_resolution: String,
    pub screen_image_format: String,
    pub user_control_mode: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sessions {
    pub max_sessions: usize,
    pub idle_timeout_secs: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Screenshots {
    pub save_dir: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8765".into(),
            agent_access: AgentAccess::default(),
            sessions: Sessions::default(),
            screenshots: Screenshots::default(),
        }
    }
}
impl Default for AgentAccess {
    fn default() -> Self {
        Self {
            computer_vision: true,
            computer_input: true,
            forward_mcp_tools: false,
            screen_resolution: "1280x720".into(),
            screen_image_format: "png".into(),
            user_control_mode: "disabled".into(),
        }
    }
}
impl Default for Sessions {
    fn default() -> Self {
        Self {
            max_sessions: 1,
            idle_timeout_secs: 780,
        }
    }
}
impl Config {
    pub fn parse(s: &str) -> Result<Self, String> {
        let c: Self = toml::from_str(s).map_err(|e| e.to_string())?;
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<(), String> {
        let bind: std::net::SocketAddr = self
            .bind
            .parse()
            .map_err(|_| "bind must be a concrete IP socket address")?;
        if bind.ip().is_unspecified() {
            return Err(
                "NO AUTHENTICATION: wildcard binds are unsafe; choose a concrete IP address".into(),
            );
        }
        if self.agent_access.computer_input && !self.agent_access.computer_vision {
            return Err("computer_input requires computer_vision".into());
        }
        if !["native", "1280x720"].contains(&self.agent_access.screen_resolution.as_str())
            || !["png", "jpeg"].contains(&self.agent_access.screen_image_format.as_str())
            || !["view_only", "view_stop", "disabled"]
                .contains(&self.agent_access.user_control_mode.as_str())
        {
            return Err("invalid agent_access enum value".into());
        }
        if self.sessions.max_sessions == 0 || self.sessions.idle_timeout_secs == 0 {
            return Err("session limits must be positive".into());
        }
        Ok(())
    }
}
