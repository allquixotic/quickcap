use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, path::Path};

pub const ACTION_LABELS: [&str; 6] = [
    "Desktop → clipboard",
    "Desktop → JPEG",
    "Region → clipboard",
    "Region → JPEG",
    "Desktop → MP4",
    "Region → MP4",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub hotkeys: Hotkeys,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hotkeys {
    pub desktop_clipboard: String,
    pub desktop_file: String,
    pub region_clipboard: String,
    pub region_file: String,
    pub desktop_video: String,
    pub region_video: String,
}

impl Default for Config {
    fn default() -> Self {
        let key = |n| format!("Ctrl+Alt+Shift+{n}");
        Self {
            hotkeys: Hotkeys {
                desktop_clipboard: key(1),
                desktop_file: key(2),
                region_clipboard: key(3),
                region_file: key(4),
                desktop_video: key(5),
                region_video: key(6),
            },
        }
    }
}

impl Config {
    pub fn strings(&self) -> [&str; 6] {
        let h = &self.hotkeys;
        [
            &h.desktop_clipboard,
            &h.desktop_file,
            &h.region_clipboard,
            &h.region_file,
            &h.desktop_video,
            &h.region_video,
        ]
    }
    pub fn bindings(&self) -> Result<Vec<Binding>> {
        let mut seen = HashSet::new();
        self.strings()
            .into_iter()
            .map(|s| {
                let binding = Binding::parse(s)?;
                ensure!(seen.insert(binding), "Duplicate hotkey: {s}");
                Ok(binding)
            })
            .collect()
    }
    pub fn load(path: &Path) -> Result<Self> {
        let config = match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("Invalid config.json")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let config = Self::default();
                // create_new ensures another instance/user edit never gets overwritten.
                use std::io::Write;
                match fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                {
                    Ok(mut f) => f.write_all(serde_json::to_string_pretty(&config)?.as_bytes())?,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        return Self::load(path);
                    }
                    Err(e) => return Err(e.into()),
                }
                config
            }
            Err(e) => return Err(e.into()),
        };
        config.bindings()?;
        Ok(config)
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct Binding {
    pub modifiers: u32,
    pub key: u32,
}

impl Binding {
    pub fn parse(text: &str) -> Result<Self> {
        let mut modifiers = 0;
        let mut key = None;
        for part in text.split('+').map(|s| s.trim().to_ascii_uppercase()) {
            let modifier = match part.as_str() {
                "ALT" => 1,
                "CTRL" | "CONTROL" => 2,
                "SHIFT" => 4,
                "WIN" => 8,
                _ => 0,
            };
            if modifier != 0 {
                ensure!(modifiers & modifier == 0, "Repeated modifier in {text}");
                modifiers |= modifier;
            } else {
                ensure!(key.is_none(), "Multiple keys in {text}");
                key = Some(
                    if let Some(number) = part.strip_prefix('F').and_then(|s| s.parse::<u32>().ok())
                    {
                        ensure!((1..=24).contains(&number), "Expected F1–F24 in {text}");
                        0x70 + number - 1
                    } else if part.len() == 1 && part.as_bytes()[0].is_ascii_alphanumeric() {
                        part.as_bytes()[0] as u32
                    } else {
                        bail!("Unsupported key in {text}; use A–Z, 0–9, F1–F24")
                    },
                );
            }
        }
        ensure!(
            modifiers & (1 | 2 | 8) != 0,
            "Hotkey needs Ctrl, Alt or Win: {text}"
        );
        Ok(Self {
            modifiers,
            key: key.context("Hotkey missing key")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_use_number_row_with_all_three_modifiers() {
        let b = Config::default().bindings().unwrap();
        assert_eq!(b.len(), 6);
        for (binding, key) in b.iter().zip(b'1'..=b'6') {
            assert_eq!(binding.modifiers, 7);
            assert_eq!(binding.key, u32::from(key));
        }
    }
    #[test]
    fn duplicate_and_malformed_bindings_rejected() {
        let mut c = Config::default();
        c.hotkeys.region_video = c.hotkeys.desktop_video.clone();
        assert!(c.bindings().is_err());
        for text in [
            "F1",
            "Ctrl+F25",
            "Ctrl+X+Y",
            "Ctrl+Ctrl+F1",
            "Ctrl+",
            "Shift+A",
        ] {
            assert!(Binding::parse(text).is_err(), "{text}");
        }
        assert_eq!(Binding::parse(" alt + CTRL + f13 ").unwrap().key, 0x7c);
    }
    #[test]
    fn unknown_config_fields_rejected() {
        let text = serde_json::to_string(&Config::default())
            .unwrap()
            .replace("\"hotkeys\"", "\"typo\"");
        assert!(serde_json::from_str::<Config>(&text).is_err());
    }
}
