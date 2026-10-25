//! The engine: scanning, payload discovery, and the journaled install and
//! restore. Nothing here touches the UI; progress is reported as [`Event`]s.

pub mod apply;
pub mod guards;
pub mod history;
pub mod ini;
pub mod journal;
pub mod payload;
pub mod pe;
pub mod scan;

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Level {
    Info,
    Ok,
    Warn,
    Error,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub level: Level,
    pub text: String,
}

impl Event {
    pub fn info(text: impl Into<String>) -> Self {
        Event { level: Level::Info, text: text.into() }
    }
    pub fn ok(text: impl Into<String>) -> Self {
        Event { level: Level::Ok, text: text.into() }
    }
    pub fn warn(text: impl Into<String>) -> Self {
        Event { level: Level::Warn, text: text.into() }
    }
}

/// An install/restore failure: a stable code plus whatever detail helps
/// (usually the file path involved).
#[derive(Debug)]
pub struct Error {
    pub code: &'static str,
    pub detail: String,
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn fail(code: &'static str, detail: impl Into<String>) -> Error {
    Error { code, detail: detail.into() }
}

impl Error {
    pub fn message(&self) -> &'static str {
        match self.code {
            "errAntiCheatConsent" => "Anti-cheat was detected. Review the warning and explicitly accept the risk to continue.",
            "errManagedModpack" => "A Mod Organizer Stock Game / Root Builder installation was detected. Direct injection could conflict with its managed ENB/ReShade, so automatic installation is blocked. Restore originals remains available.",
            "errLoaderConflict" => "A conflicting graphics loader or ReShade hook is present. No conflicting file was overwritten. Restore the previous install, or manage the external ENB/DXVK/ReShade through its own installer before retrying.",
            "errRuntimeArchitecture" => "An existing DLSS runtime beside the selected executable is unreadable or has a different architecture. It was not overwritten.",
            "errReShadeArchitecture" => "The bundled ReShade / runtime build does not match the game's architecture.",
            "errBackupInvalid" => "The original backup is incomplete or invalid. No full restore was performed. Keep _DLSS5_Backup intact and repair the game with its launcher if an original is lost.",
            "errGameRunning" => "Close the game and its helper processes before installing or restoring.",
            "errJobBusy" => "Another installation or restore is in progress. Wait for it to finish.",
            "errBackendRecovery" => "An interrupted install needs recovery. Close the game, then click Restore originals. Keep the backup folder intact.",
            "errNoWriteAccess" => "Windows will not let this app write to the game folder, so nothing was changed. Run NeuralSwap as administrator, or move the game out of Program Files.",
            "errMultipassMissing" => "This payload does not carry the RenoDX DLSS Tool (multipass) add-on.",
            "errNoNeuralRuntime" => "The payload has no nvngx_dlssnr.dll (DLSS 5 neural rendering runtime).",
            "errNoPayload" => "No DLSS 5 payload folder was found. Point Settings → Payload folder at a folder containing streamline\\ and the RenoDX add-on.",
            "errNoBackup" => "There is no backup for this game, so there is nothing to restore.",
            "errReShadeInstall" => "ReShade could not be installed beside the game executable.",
            "errReShadeSetupMissing" => "No ReShade add-on build is available in the payload.",
            "errUnsafeTarget" => "Refused to write outside the game folder (or through a symlink/junction).",
            "errFeederSupportMissing" => "The DLSS5-Feeder files in the payload are missing or failed verification.",
            "errFeederApiUnsupported" => "This build installs DLSS5-Feeder for 64-bit DirectX 11/12 games only.",
            "errShaderInstall" => "The Feeder shaders were not deployed completely.",
            "errNoExecutable" => "No game executable was found in this folder.",
            "errRouteUnsupported" => "That installation route is not available for this executable.",
            "errIo" => "A file operation failed.",
            _ => "Unexpected error.",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.detail.is_empty() {
            write!(f, "{}", self.message())
        } else {
            write!(f, "{} ({})", self.message(), self.detail)
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        fail("errIo", e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        fail("errBackupInvalid", e.to_string())
    }
}
