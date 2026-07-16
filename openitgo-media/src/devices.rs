//! Audio output device records (FFI-free) and their parsing logic.
//! Same layering as tracks.rs: player.rs fills RawAudioDevice from mpv
//! nodes; everything downstream stays testable.

/// Intermediate, FFI-free device record.
#[derive(Debug, Clone, PartialEq)]
pub struct RawAudioDevice {
    pub name: String,
    pub description: Option<String>,
}

/// An audio output device as presented to the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioDevice {
    /// Value for mpv's `audio-device` property ("auto" = follow the system).
    pub name: String,
    /// Human-readable description; `label()` falls back to `name` when None.
    pub description: Option<String>,
}

impl AudioDevice {
    pub fn label(&self) -> String {
        self.description
            .clone()
            .unwrap_or_else(|| self.name.clone())
    }
}

/// Drops empty names and duplicates (keeping the first occurrence).
pub fn parse_audio_devices(raw: Vec<RawAudioDevice>) -> Vec<AudioDevice> {
    let mut seen = std::collections::HashSet::new();
    raw.into_iter()
        .filter(|d| !d.name.is_empty())
        .filter(|d| seen.insert(d.name.clone()))
        .map(|d| AudioDevice {
            name: d.name,
            description: d.description,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(name: &str, desc: Option<&str>) -> RawAudioDevice {
        RawAudioDevice {
            name: name.to_string(),
            description: desc.map(|s| s.to_string()),
        }
    }

    #[test]
    fn parse_drops_empty_names_and_duplicates() {
        let devices = parse_audio_devices(vec![
            raw("coreaudio/AG06", Some("AG06/AG03")),
            raw("", Some("empty")),
            raw("coreaudio/AG06", Some("dup")),
            raw("coreaudio/hdmi", None),
        ]);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].name, "coreaudio/AG06");
        assert_eq!(devices[1].name, "coreaudio/hdmi");
    }

    #[test]
    fn label_falls_back_to_name_without_description() {
        let d = AudioDevice {
            name: "coreaudio/hdmi".into(),
            description: None,
        };
        assert_eq!(d.label(), "coreaudio/hdmi");
        let d2 = AudioDevice {
            name: "coreaudio/AG06".into(),
            description: Some("AG06/AG03".into()),
        };
        assert_eq!(d2.label(), "AG06/AG03");
    }
}
