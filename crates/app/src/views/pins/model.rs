//! Strict consumers of the pinned engine's pin tool results; never interpret output as paths.
use autocraft_core::{Error, Result, ToolReply};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Mode {
    Frozen,
    Light,
    Live,
}
impl Mode {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Frozen => "app.pins.frozen",
            Self::Light => "app.pins.light",
            Self::Live => "app.pins.live",
        }
    }
    fn parameter(self) -> &'static str {
        match self {
            Self::Frozen => "frozen",
            Self::Light => "light",
            Self::Live => "live",
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
pub(super) struct Pin {
    pub id: u32,
    pub name: String,
    pub mode: Mode,
    pub round: Option<u32>,
    pub bytes: u64,
    pub shared_bytes: u64,
}
#[derive(Clone, Debug, Deserialize)]
pub(super) struct Row {
    pub pin: Pin,
    pub running: bool,
}
#[derive(Debug, Deserialize)]
pub(super) struct Listing {
    pub count: usize,
    pub pins: Vec<Row>,
    pub copied_bytes: u64,
}
#[derive(Clone, Copy, Debug, Deserialize)]
pub(super) struct Estimate {
    pub bytes: u64,
    pub light_bytes: u64,
    pub files: u64,
}
#[derive(Clone, Debug)]
pub(super) enum Action {
    Create { mode: Mode, name: String },
    Open { pin: u32, another: bool },
    Stop(u32),
    Rename { pin: u32, name: String },
    Delete(u32),
    Folder(u32),
}
impl Action {
    pub(super) fn tool(&self) -> &'static str {
        match self {
            Self::Create { .. } => "project_pin_create",
            Self::Open { .. } => "project_pin_open",
            Self::Stop(_) => "project_pin_stop",
            Self::Rename { .. } => "project_pin_rename",
            Self::Delete(_) => "project_pin_delete",
            Self::Folder(_) => "project_open_folder",
        }
    }
    pub(super) fn params(&self, project: &str) -> Value {
        match self {
            Self::Create { mode, name } => json!({"project": project, "mode": mode.parameter(),
                "name": if name.trim().is_empty() { None } else { Some(name.trim()) }}),
            Self::Open { pin, another } => {
                json!({"project": project, "pin": pin, "another": another})
            }
            Self::Stop(pin) => json!({"project": project, "pin": pin}),
            Self::Rename { pin, name } => json!({"project": project, "pin": pin, "name": name}),
            Self::Delete(pin) => json!({"project": project, "pin": pin, "confirm": true}),
            Self::Folder(pin) => json!({"project": project, "pin": pin}),
        }
    }
    pub(super) fn confirmation(&self) -> &'static str {
        match self {
            Self::Create { .. } => "app.pins.create_confirm",
            Self::Open { another: false, .. } => "app.pins.open_confirm",
            Self::Open { another: true, .. } => "app.pins.another_confirm",
            Self::Stop(_) => "app.pins.stop_confirm",
            Self::Rename { .. } => "app.pins.rename_confirm",
            Self::Delete(_) => "app.pins.delete_confirm",
            Self::Folder(_) => "app.pins.folder_confirm",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Created,
    Running,
    AlreadyRunning,
    Closed,
    Stopped(u64),
    Renamed,
    Deleted,
    Folder,
}
fn value(reply: ToolReply) -> Result<Value> {
    match reply {
        ToolReply::Value(value) => Ok(value),
        ToolReply::Evidence { .. } => Err(invalid()),
    }
}
fn invalid() -> Error {
    Error::protocol("固定版本结果格式无效")
}
pub(super) fn listing(reply: ToolReply) -> Result<Listing> {
    let list: Listing = serde_json::from_value(value(reply)?).map_err(|_| invalid())?;
    if list.count != list.pins.len()
        || list.pins.iter().any(|row| row.pin.id == 0)
        || list.pins.iter().enumerate().any(|(i, row)| {
            list.pins[..i]
                .iter()
                .any(|earlier| earlier.pin.id == row.pin.id)
        })
    {
        return Err(invalid());
    }
    Ok(list)
}
pub(super) fn estimate(reply: ToolReply) -> Result<Estimate> {
    let value = value(reply)?;
    serde_json::from_value(value.get("estimate").cloned().ok_or_else(invalid)?)
        .map_err(|_| invalid())
}
pub(super) fn outcome(action: &Action, reply: ToolReply) -> Result<Outcome> {
    let value = value(reply)?;
    let valid_pin = |id| value.get("pin").and_then(Value::as_u64) == Some(u64::from(id));
    match action {
        Action::Create { .. } | Action::Rename { .. } => {
            let pin: Pin = serde_json::from_value(value.get("pin").cloned().ok_or_else(invalid)?)
                .map_err(|_| invalid())?;
            match action {
                Action::Create { .. } if pin.id > 0 => Ok(Outcome::Created),
                Action::Rename { pin: id, .. } if *id == pin.id => Ok(Outcome::Renamed),
                Action::Create { .. }
                | Action::Rename { .. }
                | Action::Open { .. }
                | Action::Stop(_)
                | Action::Delete(_)
                | Action::Folder(_) => Err(invalid()),
            }
        }
        Action::Open { pin, .. } if valid_pin(*pin) => {
            match value.get("already_running").and_then(Value::as_bool) {
                Some(true) => Ok(Outcome::AlreadyRunning),
                Some(false) => match value.get("closed_at_once") {
                    Some(Value::Null) => Ok(Outcome::Running),
                    Some(closed) if closed.get("code").and_then(Value::as_i64) == Some(0) => {
                        Ok(Outcome::Closed)
                    }
                    _ => Err(invalid()),
                },
                None => Err(invalid()),
            }
        }
        Action::Stop(pin) if valid_pin(*pin) => value
            .get("stopped")
            .and_then(Value::as_u64)
            .map(Outcome::Stopped)
            .ok_or_else(invalid),
        Action::Delete(pin)
            if value.get("removed").and_then(Value::as_u64) == Some(u64::from(*pin)) =>
        {
            Ok(Outcome::Deleted)
        }
        Action::Folder(_) if value.get("opened").and_then(Value::as_str).is_some() => {
            Ok(Outcome::Folder)
        }
        Action::Open { .. } | Action::Stop(_) | Action::Delete(_) | Action::Folder(_) => {
            Err(invalid())
        }
    }
}

pub(super) fn size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.)
    } else if bytes >= 1_000_000 {
        format!("{} MB", bytes.div_ceil(1_000_000))
    } else {
        format!("{} KB", bytes.div_ceil(1_000))
    }
}
