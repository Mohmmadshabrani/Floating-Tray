use crate::protocol::{self, Change, IconImage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

impl Edge {
    pub const ALL: [Self; 4] = [Self::Left, Self::Top, Self::Right, Self::Bottom];

    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Top => "Top",
            Self::Right => "Right",
            Self::Bottom => "Bottom",
        }
    }

    pub fn vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    pub fn anchor(self, work: (i32, i32, i32, i32), size: (i32, i32), gap: i32) -> (i32, i32) {
        let (left, top, right, bottom) = work;
        let (width, height) = size;
        let cx = left + (right - left - width) / 2;
        let cy = top + (bottom - top - height) / 2;
        match self {
            Self::Left => (left + gap, cy),
            Self::Top => (cx, top + gap),
            Self::Right => (right - width - gap, cy),
            Self::Bottom => (cx, bottom - height - gap),
        }
    }
}

pub struct Settings {
    pub edge: Edge,
    pub pins: Vec<String>,
}

impl Settings {
    pub fn parse(text: &str) -> Self {
        let mut settings = Self {
            edge: Edge::Right,
            pins: Vec::new(),
        };
        for (key, value) in text.lines().filter_map(|line| line.split_once('=')) {
            match key {
                "edge" => {
                    if let Some(edge) = Edge::ALL.into_iter().find(|edge| edge.name() == value) {
                        settings.edge = edge;
                    }
                }
                "pin" if !value.is_empty() && !settings.pins.iter().any(|pin| pin == value) => {
                    settings.pins.push(value.to_owned());
                }
                _ => {}
            }
        }
        settings
    }

    pub fn encode(&self) -> String {
        let mut text = format!("edge={}\n", self.edge.name());
        for pin in &self.pins {
            text.push_str(&format!("pin={pin}\n"));
        }
        text
    }

    pub fn toggle(&mut self, key: &str) {
        if let Some(index) = self.pins.iter().position(|pin| pin == key) {
            self.pins.remove(index);
        } else {
            self.pins.push(key.to_owned());
        }
    }
}

pub struct Item {
    pub window: u32,
    pub id: u32,
    pub callback: u32,
    pub version: u32,
    pub hidden: bool,
    pub tooltip: String,
    pub exe: String,
    pub guid: Option<[u8; 16]>,
    pub image: Option<IconImage>,
    pub key: Option<String>,
}

impl Item {
    fn make_pin_key(&self) -> Option<String> {
        if let Some(guid) = self.guid {
            let hex: String = guid.iter().map(|byte| format!("{byte:02x}")).collect();
            return Some(format!("guid:{hex}"));
        }
        if self.exe.is_empty() {
            return None;
        }
        Some(format!("exe:{}:{}", self.exe, self.id))
    }

    pub fn label(&self) -> &str {
        if !self.tooltip.is_empty() {
            &self.tooltip
        } else {
            self.exe
                .rsplit('\\')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("Tray icon")
        }
    }
}

#[derive(Default)]
pub struct Tray {
    pub items: Vec<Item>,
}

impl Tray {
    pub fn apply(&mut self, change: Change, resolve_exe: impl Fn(u32) -> Option<String>) {
        let index = change
            .guid
            .and_then(|guid| self.items.iter().position(|item| item.guid == Some(guid)))
            .or_else(|| {
                self.items
                    .iter()
                    .position(|item| item.window == change.window && item.id == change.id)
            });
        if change.operation == protocol::DELETE {
            if let Some(index) = index {
                self.items.remove(index);
            }
            return;
        }
        let item = if let Some(index) = index {
            &mut self.items[index]
        } else {
            if change.window == 0 || self.items.len() >= 512 {
                return;
            }
            self.items.push(Item {
                window: change.window,
                id: change.id,
                callback: 0,
                version: 0,
                hidden: false,
                tooltip: String::new(),
                exe: resolve_exe(change.window).unwrap_or_default(),
                guid: None,
                image: None,
                key: None,
            });
            self.items.last_mut().unwrap()
        };
        let owner_changed = item.window != change.window && change.window != 0;
        if owner_changed {
            item.window = change.window;
            item.id = change.id;
            item.exe = resolve_exe(change.window).unwrap_or_default();
        }
        if item.key.is_none()
            || owner_changed
            || (change.guid.is_some() && item.guid != change.guid)
        {
            item.guid = change.guid.or(item.guid);
            item.key = item.make_pin_key();
        }
        if let Some(version) = change.version {
            item.version = version.min(4);
            return;
        }
        if change.flags & protocol::MESSAGE != 0 {
            item.callback = change.callback;
        }
        if change.flags & protocol::TIP != 0 {
            item.tooltip = change.tooltip;
        }
        if change.flags & protocol::STATE != 0 {
            item.hidden = change.hidden;
        }
        // Many NIM_MODIFY messages only update a tooltip, callback or state.
        if let Some(image) = change.image {
            item.image = Some(image);
        }
    }

    pub fn visible(&self, settings: &Settings) -> (Vec<usize>, Vec<usize>) {
        let mut pinned = Vec::new();
        let mut overflow = Vec::new();
        for (index, item) in self.items.iter().enumerate() {
            if item.hidden || item.image.is_none() {
                continue;
            }
            let priority = item
                .key
                .as_ref()
                .and_then(|key| settings.pins.iter().position(|pin| pin == key));
            if let Some(priority) = priority {
                pinned.push((priority, index));
            } else {
                overflow.push(index);
            }
        }
        pinned.sort_unstable();
        (
            pinned.into_iter().map(|(_, index)| index).collect(),
            overflow,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(operation: u32) -> Change {
        Change {
            operation,
            window: 11,
            id: 2,
            flags: 0,
            callback: 0,
            tooltip: String::new(),
            hidden: false,
            version: None,
            guid: None,
            image: None,
        }
    }

    #[test]
    fn centers_the_dock_on_all_four_edges() {
        let area = (0, 0, 1920, 1040);
        assert!(Edge::Right.vertical());
        assert!(!Edge::Bottom.vertical());
        assert_eq!(Edge::Right.anchor(area, (40, 100), 8), (1872, 470));
        assert_eq!(Edge::Left.anchor(area, (40, 100), 8), (8, 470));
        assert_eq!(Edge::Top.anchor(area, (100, 40), 8), (910, 8));
        assert_eq!(Edge::Bottom.anchor(area, (100, 40), 8), (910, 992));
    }

    #[test]
    fn partial_updates_preserve_live_icon_and_pin() {
        let mut tray = Tray::default();
        let mut add = change(protocol::ADD);
        add.flags = protocol::MESSAGE;
        add.callback = 0x8001;
        add.image = Some(IconImage {
            width: 12,
            height: 12,
            rgba: vec![0; 576],
        });
        tray.apply(add, |_| Some(r"c:\apps\slack.exe".into()));
        let mut tip = change(protocol::MODIFY);
        tip.flags = protocol::TIP;
        tip.tooltip = "Slack • 1 unread".into();
        tray.apply(tip, |_| {
            panic!("a known icon should not need its path resolved")
        });
        let mut settings = Settings::parse("");
        settings.toggle(tray.items[0].key.as_deref().unwrap());
        assert_eq!(tray.visible(&settings), (vec![0], vec![]));
        assert_eq!(tray.items[0].label(), "Slack • 1 unread");
        assert_eq!(tray.items[0].callback, 0x8001);
        let image = tray.items[0].image.as_ref().unwrap();
        assert_eq!((image.width, image.height), (12, 12));
        let mut unread = change(protocol::MODIFY);
        unread.flags = protocol::IMAGE;
        unread.image = Some(IconImage {
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 255],
        });
        tray.apply(unread, |_| None);
        assert_eq!(tray.items[0].image.as_ref().unwrap().rgba, [255, 0, 0, 255]);
        assert_eq!(tray.visible(&settings), (vec![0], vec![]));
        tray.apply(change(protocol::DELETE), |_| None);
        assert!(tray.items.is_empty());
        assert_eq!(Settings::parse(&settings.encode()).pins, settings.pins);
    }
}
