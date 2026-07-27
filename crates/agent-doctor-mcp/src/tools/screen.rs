// Screen MCP — macOS Accessibility API window/element reading (placeholder)
// Will be implemented in a follow-up. Architecture: define a ScreenContext similar to
// BrowserContext, with tools like screen_get_ui_tree, screen_click, screen_type, etc.

use anyhow::Result;

pub struct ScreenContext;

impl ScreenContext {
    pub fn connect() -> Result<Self> {
        anyhow::bail!("Screen MCP not yet implemented");
    }
}
