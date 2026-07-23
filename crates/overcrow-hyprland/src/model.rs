use overcrow_protocol::Rect;
use serde::Deserialize;

pub const OVERLAY_APP_ID: &str = "io.github.overcrow.Overlay";

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct HyprWindow {
    pub address: String,
    #[serde(default)]
    pub mapped: bool,
    #[serde(default)]
    pub hidden: bool,
    pub at: [i64; 2],
    pub size: [i64; 2],
    pub monitor: i32,
    pub class: String,
    #[serde(default)]
    pub title: String,
    pub pid: i64,
    #[serde(default)]
    pub workspace: Option<HyprWorkspace>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl HyprWindow {
    pub fn rect(&self) -> Option<Rect> {
        let x = i32::try_from(self.at[0]).ok()?;
        let y = i32::try_from(self.at[1]).ok()?;
        let width = u32::try_from(self.size[0])
            .ok()
            .filter(|value| *value > 0 && i32::try_from(*value).is_ok())?;
        let height = u32::try_from(self.size[1])
            .ok()
            .filter(|value| *value > 0 && i32::try_from(*value).is_ok())?;
        Some(Rect {
            x,
            y,
            width,
            height,
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct HyprWorkspace {
    pub id: i32,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct HyprMonitor {
    pub id: i32,
    pub scale: f64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowAddress(String);

impl WindowAddress {
    pub fn parse(value: &str) -> Option<Self> {
        let digits = value.strip_prefix("0x")?;
        if digits.is_empty()
            || digits.len() > 16
            || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        Some(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowReport {
    pub address: WindowAddress,
    pub pid: i32,
    pub title: String,
    pub app_id: String,
    pub rect: Rect,
    pub scale: f64,
    pub workspace_id: i32,
}

impl WindowReport {
    pub fn from_window(window: &HyprWindow, monitors: &[HyprMonitor]) -> Option<Self> {
        if !window.mapped || window.hidden || window.class.is_empty() {
            return None;
        }
        let address = WindowAddress::parse(&window.address)?;
        let pid = i32::try_from(window.pid).ok().filter(|pid| *pid > 0)?;
        let rect = window.rect()?;
        let scale = monitors
            .iter()
            .find(|monitor| monitor.id == window.monitor)?
            .scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let workspace_id = window
            .workspace
            .as_ref()
            .map(|workspace| workspace.id)
            .filter(|id| *id > 0)?;

        Some(Self {
            address,
            pid,
            title: window.title.clone(),
            app_id: window.class.clone(),
            rect,
            scale,
            workspace_id,
        })
    }

    pub fn is_overlay(&self) -> bool {
        self.app_id == OVERLAY_APP_ID
    }
}

#[cfg(test)]
mod tests {
    use overcrow_protocol::Rect;

    use super::{HyprMonitor, HyprWindow, OVERLAY_APP_ID, WindowReport};

    fn sample_window(address: &str, class: &str, monitor: i32) -> HyprWindow {
        HyprWindow {
            address: address.to_owned(),
            mapped: true,
            hidden: false,
            at: [12, 36],
            size: [2417, 1680],
            monitor,
            class: class.to_owned(),
            title: "Pal".to_owned(),
            pid: 136_279,
            workspace: Some(super::HyprWorkspace {
                id: 1,
                name: "1".to_owned(),
            }),
            tags: Vec::new(),
        }
    }

    #[test]
    fn normalizes_a_mapped_xwayland_game() {
        let window: HyprWindow = serde_json::from_str(
            r#"{
                "address":"0x55aabb", "mapped":true, "hidden":false,
                "at":[12,36], "size":[2417,1680], "monitor":1,
                "class":"steam_app_1623730", "title":"Pal  ", "pid":136279,
                "workspace":{"id":3,"name":"3"},
                "tags":["game-window"]
            }"#,
        )
        .expect("window JSON should decode");
        let monitors = [HyprMonitor { id: 1, scale: 1.25 }];

        let report = WindowReport::from_window(&window, &monitors).expect("valid report");

        assert_eq!(report.address.as_str(), "0x55aabb");
        assert_eq!(report.pid, 136_279);
        assert_eq!(
            report.rect,
            Rect {
                x: 12,
                y: 36,
                width: 2417,
                height: 1680,
            }
        );
        assert_eq!(report.scale, 1.25);
        assert_eq!(report.workspace_id, 3);
        assert_eq!(window.tags, vec!["game-window"]);
        assert!(!report.is_overlay());
    }

    #[test]
    fn rejects_missing_or_non_positive_workspace_ids() {
        let monitors = [HyprMonitor { id: 0, scale: 1.0 }];
        for workspace in [
            String::new(),
            r#", "workspace":{"id":0,"name":"invalid"}"#.to_owned(),
            r#", "workspace":{"id":-99,"name":"special:invalid"}"#.to_owned(),
        ] {
            let json = format!(
                r#"{{"address":"0x1","mapped":true,"hidden":false,"at":[0,0],"size":[100,100],"monitor":0,"class":"game","title":"g","pid":1{workspace}}}"#
            );
            let window: HyprWindow = serde_json::from_str(&json).expect("fixture should decode");
            assert!(WindowReport::from_window(&window, &monitors).is_none());
        }
    }

    #[test]
    fn rejects_hidden_unmapped_and_invalid_windows() {
        for json in [
            r#"{"address":"0x1","mapped":false,"hidden":false,"at":[0,0],"size":[100,100],"monitor":0,"class":"game","title":"g","pid":1,"workspace":{"id":1,"name":"1"}}"#,
            r#"{"address":"0x1","mapped":true,"hidden":true,"at":[0,0],"size":[100,100],"monitor":0,"class":"game","title":"g","pid":1,"workspace":{"id":1,"name":"1"}}"#,
            r#"{"address":"bad","mapped":true,"hidden":false,"at":[0,0],"size":[100,100],"monitor":0,"class":"game","title":"g","pid":1,"workspace":{"id":1,"name":"1"}}"#,
            r#"{"address":"0x1","mapped":true,"hidden":false,"at":[0,0],"size":[0,100],"monitor":0,"class":"game","title":"g","pid":1,"workspace":{"id":1,"name":"1"}}"#,
            r#"{"address":"0x1","mapped":true,"hidden":false,"at":[0,0],"size":[100,100],"monitor":0,"class":"","title":"g","pid":1,"workspace":{"id":1,"name":"1"}}"#,
        ] {
            let window: HyprWindow = serde_json::from_str(json).expect("fixture should decode");
            assert!(
                WindowReport::from_window(&window, &[HyprMonitor { id: 0, scale: 1.0 }]).is_none()
            );
        }
    }

    #[test]
    fn requires_a_finite_positive_matching_monitor_scale() {
        let window = sample_window("0x1", "game", 0);
        for monitors in [
            Vec::new(),
            vec![HyprMonitor { id: 1, scale: 1.0 }],
            vec![HyprMonitor { id: 0, scale: 0.0 }],
            vec![HyprMonitor {
                id: 0,
                scale: f64::NAN,
            }],
        ] {
            assert!(WindowReport::from_window(&window, &monitors).is_none());
        }
    }

    #[test]
    fn identifies_only_the_exact_overlay_class() {
        let monitors = [HyprMonitor { id: 0, scale: 1.0 }];
        assert!(
            WindowReport::from_window(&sample_window("0x2", OVERLAY_APP_ID, 0), &monitors)
                .expect("overlay should normalize")
                .is_overlay()
        );
        assert!(
            !WindowReport::from_window(
                &sample_window("0x3", "io.github.overcrow.Overlay.fake", 0),
                &monitors,
            )
            .expect("lookalike should normalize")
            .is_overlay()
        );
    }

    #[test]
    fn exposes_only_positive_i32_compatible_live_rectangles() {
        let valid = sample_window("0x1", "game", 0);
        assert_eq!(
            valid.rect(),
            Some(Rect {
                x: 12,
                y: 36,
                width: 2417,
                height: 1680,
            })
        );

        let mut zero_width = valid.clone();
        zero_width.size[0] = 0;
        assert_eq!(zero_width.rect(), None);

        let mut overflowing = valid;
        overflowing.size[1] = i64::from(i32::MAX) + 1;
        assert_eq!(overflowing.rect(), None);
    }
}
