//! Logitech G HUB local WebSocket API (`ws://127.0.0.1:9010`).
//!
//! When G HUB is running it usually owns the receivers, and its numbers are exact. The
//! protocol logic lives in [`Session`] (pure, unit-tested); [`run`] is the socket loop.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::Message;

use crate::model::{Kind, Link, Reading};
use crate::names::kind_from_name;
use crate::wake::Wake;

const ADDRESS: ([u8; 4], u16) = ([127, 0, 0, 1], 9010);
const POLL_INTERVAL: Duration = Duration::from_secs(60);
const RETRY_MIN: Duration = Duration::from_secs(3);
const RETRY_MAX: Duration = Duration::from_secs(60);

fn message(verb: &str, path: &str, id: &str) -> String {
    json!({ "msgId": id, "verb": verb, "path": path }).to_string()
}

struct Info {
    id: String,
    unit: String,
    name: String,
    kind: Kind,
    link: Link,
    connected: bool,
}

struct Level {
    percent: u8,
    charging: bool,
    full: bool,
}

/// Protocol state of one G HUB connection.
#[derive(Default)]
pub struct Session {
    devices: Vec<Info>,
    battery: HashMap<String, Level>,
}

impl Session {
    /// Messages to send right after the socket opens.
    pub fn greeting() -> Vec<String> {
        vec![
            message("SUBSCRIBE", "/battery/state/changed", ""),
            message("SUBSCRIBE", "/devices/state/changed", ""),
            Self::list_request(),
        ]
    }

    pub fn list_request() -> String {
        message("GET", "/devices/list", "list")
    }

    /// Process one incoming text frame. Returns the messages to send in response and
    /// whether the device list may have changed.
    pub fn handle(&mut self, text: &str) -> (Vec<String>, bool) {
        let Ok(msg) = serde_json::from_str::<Value>(text) else {
            return (Vec::new(), false);
        };
        let path = msg["path"].as_str().unwrap_or("");
        let payload = &msg["payload"];
        if path == "/devices/list" {
            let mut outgoing = Vec::new();
            self.devices.clear();
            for d in payload["deviceInfos"].as_array().into_iter().flatten() {
                let Some(info) = parse_info(d) else { continue };
                if d["capabilities"]["hasBatteryStatus"]
                    .as_bool()
                    .unwrap_or(true)
                {
                    outgoing.push(message("GET", &format!("/battery/{}/state", info.id), ""));
                }
                self.devices.push(info);
            }
            let devices = &self.devices;
            self.battery
                .retain(|id, _| devices.iter().any(|d| d.id == *id));
            (outgoing, true)
        } else if path.starts_with("/devices/") && path.ends_with("/changed") {
            (vec![Self::list_request()], false)
        } else if path.starts_with("/battery/") {
            let (Some(id), Some(percent)) =
                (payload["deviceId"].as_str(), payload["percentage"].as_f64())
            else {
                return (Vec::new(), false);
            };
            let percent = percent.clamp(0.0, 100.0) as u8;
            let level = Level {
                percent,
                charging: payload["charging"].as_bool().unwrap_or(false),
                full: payload["fullyCharged"].as_bool().unwrap_or(false) && percent >= 98,
            };
            self.battery.insert(id.to_string(), level);
            (Vec::new(), true)
        } else {
            (Vec::new(), false)
        }
    }

    pub fn readings(&self) -> Vec<Reading> {
        self.devices
            .iter()
            .map(|d| {
                let level = self.battery.get(&d.id);
                Reading::new(format!("logi:{}", d.unit), d.name.clone(), d.kind, d.link)
                    .battery(level.map(|l| l.percent))
                    .charging(level.is_some_and(|l| l.charging))
                    .full(level.is_some_and(|l| l.full))
                    .connected(d.connected)
            })
            .collect()
    }
}

fn parse_info(d: &Value) -> Option<Info> {
    let id = d["id"].as_str().filter(|s| !s.is_empty())?.to_string();
    // The unit id matches what HID++ feature 0x0003 reports, which lets both sources merge.
    let unit = match &d["deviceUnitId"] {
        Value::Number(n) => n.as_u64().map(|n| format!("{n:08X}")),
        Value::String(s) => s.parse::<u64>().ok().map(|n| format!("{n:08X}")),
        _ => None,
    }
    .unwrap_or_else(|| id.clone());
    let name = ["displayName", "extendedDisplayName"]
        .iter()
        .find_map(|key| d[*key].as_str().filter(|s| !s.is_empty()))
        .unwrap_or("Logitech G")
        .to_string();
    let kind = match d["deviceType"]
        .as_str()
        .unwrap_or("")
        .to_uppercase()
        .as_str()
    {
        "KEYBOARD" => Kind::Keyboard,
        "MOUSE" => Kind::Mouse,
        "HEADSET" | "HEADPHONES" | "AUDIO" | "GAMING_HEADSET" | "EARBUDS" | "IEM" => Kind::Headset,
        "SPEAKER" | "SPEAKERS" => Kind::Speaker,
        "GAMEPAD" | "CONTROLLER" | "GAME_CONTROLLER" | "WHEEL" | "RACING_WHEEL" | "SIMULATION"
        | "JOYSTICK" | "FLIGHT" => Kind::Gamepad,
        _ => kind_from_name(&name),
    };
    let link = ["displayConnectionType", "connectionType"]
        .iter()
        .find_map(|key| d[*key].as_str())
        .unwrap_or("");
    let link = if link.eq_ignore_ascii_case("bluetooth") {
        Link::Bluetooth
    } else if link.eq_ignore_ascii_case("usb") || link.eq_ignore_ascii_case("wired") {
        Link::Usb
    } else {
        Link::Lightspeed
    };
    let state = d["state"].as_str().unwrap_or("");
    let connected = matches!(state, "ACTIVE" | "CONNECTED" | "CHARGING" | "READY")
        || d["isConnected"].as_bool().unwrap_or(false);
    Some(Info {
        id,
        unit,
        name,
        kind,
        link,
        connected,
    })
}

fn is_timeout(error: &tungstenite::Error) -> bool {
    matches!(error, tungstenite::Error::Io(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

/// Run one connection until it breaks or the worker is asked to stop.
fn connection(wake: &Wake, emit: &dyn Fn(Vec<Reading>)) -> Result<(), tungstenite::Error> {
    let stream = TcpStream::connect_timeout(&SocketAddr::from(ADDRESS), Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut request = format!("ws://127.0.0.1:{}", ADDRESS.1).into_client_request()?;
    // G HUB only accepts its own Electron front end: local origin, "json" sub-protocol.
    request
        .headers_mut()
        .insert("Origin", HeaderValue::from_static("file://"));
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", HeaderValue::from_static("json"));
    let (mut socket, _) = tungstenite::client(request, stream).map_err(|e| match e {
        tungstenite::HandshakeError::Failure(e) => e,
        tungstenite::HandshakeError::Interrupted(_) => {
            tungstenite::Error::Io(ErrorKind::TimedOut.into())
        }
    })?;
    log::info!("connected to Logitech G HUB");
    socket
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(250)))?;

    let mut session = Session::default();
    for text in Session::greeting() {
        socket.send(Message::text(text))?;
    }
    let mut last_poll = Instant::now();
    let mut dirty = false;
    loop {
        if wake.is_stopped() {
            let _ = socket.close(None);
            return Ok(());
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                let (outgoing, changed) = session.handle(text.as_str());
                dirty |= changed;
                for text in outgoing {
                    socket.send(Message::text(text))?;
                }
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(e) if is_timeout(&e) => {
                // The socket went quiet: a good moment to publish a burst of updates at once.
                if std::mem::take(&mut dirty) {
                    emit(session.readings());
                }
                if wake.take_refresh() || last_poll.elapsed() >= POLL_INTERVAL {
                    last_poll = Instant::now();
                    socket.send(Message::text(Session::list_request()))?;
                }
            }
            Err(e) => return Err(e),
        }
    }
}

/// Worker loop: connect, stream updates, reconnect with exponential back-off.
pub fn run(wake: Arc<Wake>, emit: impl Fn(Vec<Reading>)) {
    let mut delay = RETRY_MIN;
    while !wake.is_stopped() {
        let started = Instant::now();
        let published = std::cell::Cell::new(false);
        let result = connection(&wake, &|readings| {
            published.set(true);
            emit(readings);
        });
        if let Err(e) = &result {
            log::debug!("G HUB not available: {e}");
        }
        if published.get() {
            // The connection is gone; its devices must not linger with stale values.
            emit(Vec::new());
        }
        if started.elapsed() > Duration::from_secs(30) {
            delay = RETRY_MIN;
        }
        // A manual refresh retries immediately.
        if wake.sleep(delay) {
            delay = RETRY_MIN;
        } else {
            delay = (delay * 2).min(RETRY_MAX);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{"msgId":"list","verb":"GET","path":"/devices/list","payload":{"deviceInfos":[
        {"id":"dev00000001","deviceUnitId":3735928559,"displayName":"G502 LIGHTSPEED","deviceType":"MOUSE",
         "connectionType":"WIRELESS","state":"ACTIVE","capabilities":{"hasBatteryStatus":true}},
        {"id":"dev00000002","displayName":"G915 TKL","deviceType":"KEYBOARD","connectionType":"BLUETOOTH",
         "state":"ABSENT","capabilities":{"hasBatteryStatus":true}},
        {"id":"dev00000003","displayName":"G560","deviceType":"SPEAKER","connectionType":"USB",
         "state":"ACTIVE","capabilities":{"hasBatteryStatus":false}},
        {"displayName":"broken entry"}]}}"#;

    #[test]
    fn greeting_subscribes_and_lists() {
        let g = Session::greeting();
        assert_eq!(g.len(), 3);
        assert!(g[0].contains("SUBSCRIBE") && g[0].contains("/battery/state/changed"));
        assert!(g[2].contains(r#""msgId":"list""#));
    }

    #[test]
    fn list_then_battery() {
        let mut s = Session::default();
        let (out, changed) = s.handle(LIST);
        assert!(changed);
        assert_eq!(out.len(), 2, "only devices with a battery are queried");
        assert!(out[0].contains("/battery/dev00000001/state"));

        let (_, changed) = s.handle(
            r#"{"path":"/battery/dev00000001/state","payload":{"deviceId":"dev00000001","percentage":77,"charging":true,"fullyCharged":false}}"#,
        );
        assert!(changed);

        let r = s.readings();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].id, "logi:DEADBEEF");
        assert_eq!(
            (r[0].battery, r[0].charging, r[0].connected),
            (Some(77), true, true)
        );
        assert_eq!(r[0].link, Link::Lightspeed);
        assert_eq!(
            (r[1].link, r[1].connected, r[1].battery),
            (Link::Bluetooth, false, None)
        );
        assert_eq!(r[1].id, "logi:dev00000002");
        assert_eq!(r[2].kind, Kind::Speaker);
    }

    #[test]
    fn hostile_input_is_ignored_or_clamped() {
        let mut s = Session::default();
        assert!(!s.handle("not json").1);
        assert!(
            !s.handle(r#"{"path":"/battery/x/state","payload":"nope"}"#)
                .1
        );
        s.handle(LIST);
        s.handle(r#"{"path":"/battery/dev00000001/state","payload":{"deviceId":"dev00000001","percentage":9000,"fullyCharged":true}}"#);
        let r = s.readings();
        assert_eq!(r[0].battery, Some(100));
        assert!(r[0].full);
    }

    #[test]
    fn device_change_triggers_new_list() {
        let mut s = Session::default();
        let (out, changed) = s.handle(r#"{"path":"/devices/state/changed","payload":{}}"#);
        assert!(!changed);
        assert_eq!(out, vec![Session::list_request()]);
    }
}
