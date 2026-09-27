use serde::Serialize;

/// Stable error codes the UI maps to beginner-friendly copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechErrorCode {
    Unavailable,
    PermissionDenied,
    NoSpeech,
    Busy,
    Cancelled,
    Failed,
}

impl SpeechErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::PermissionDenied => "permission_denied",
            Self::NoSpeech => "no_speech",
            Self::Busy => "busy",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SpeechError {
    pub code: SpeechErrorCode,
    pub detail: String,
}

impl SpeechError {
    pub fn new(code: SpeechErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub fn to_command_error(&self) -> String {
        format!("speech.{}:{}", self.code.as_str(), self.detail)
    }
}

impl std::fmt::Display for SpeechError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_command_error())
    }
}

impl std::error::Error for SpeechError {}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechCapability {
    pub available: bool,
    pub backend: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SpeechOptions {
    /// BCP-47 language tag, e.g. `zh-CN` / `en-US`.
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechResult {
    pub text: String,
    pub confidence: f32,
    pub is_final: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum SpeechEvent {
    #[serde(rename = "partial")]
    Partial { text: String },
    #[serde(rename = "final")]
    Final { text: String, confidence: f32 },
    #[serde(rename = "error")]
    Error { code: String, detail: String },
    #[serde(rename = "cancelled")]
    Cancelled,
}
