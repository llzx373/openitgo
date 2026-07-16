use egui::Key;

pub fn key_from_name(name: &str) -> Option<Key> {
    match name {
        "ArrowRight" => Some(Key::ArrowRight),
        "ArrowLeft" => Some(Key::ArrowLeft),
        "ArrowUp" => Some(Key::ArrowUp),
        "ArrowDown" => Some(Key::ArrowDown),
        "PageDown" => Some(Key::PageDown),
        "PageUp" => Some(Key::PageUp),
        "Space" => Some(Key::Space),
        "Enter" => Some(Key::Enter),
        "Escape" => Some(Key::Escape),
        "F11" => Some(Key::F11),
        "Plus" => Some(Key::Plus),
        "Equals" => Some(Key::Equals),
        "Minus" => Some(Key::Minus),
        "Num0" => Some(Key::Num0),
        "Num1" => Some(Key::Num1),
        "Num2" => Some(Key::Num2),
        "Num3" => Some(Key::Num3),
        "Num4" => Some(Key::Num4),
        "Num5" => Some(Key::Num5),
        "Num6" => Some(Key::Num6),
        "Num7" => Some(Key::Num7),
        "Num8" => Some(Key::Num8),
        "Num9" => Some(Key::Num9),
        "A" => Some(Key::A),
        "B" => Some(Key::B),
        "C" => Some(Key::C),
        "D" => Some(Key::D),
        "E" => Some(Key::E),
        "F" => Some(Key::F),
        "G" => Some(Key::G),
        "H" => Some(Key::H),
        "I" => Some(Key::I),
        "J" => Some(Key::J),
        "K" => Some(Key::K),
        "L" => Some(Key::L),
        "M" => Some(Key::M),
        "N" => Some(Key::N),
        "O" => Some(Key::O),
        "P" => Some(Key::P),
        "Q" => Some(Key::Q),
        "R" => Some(Key::R),
        "S" => Some(Key::S),
        "T" => Some(Key::T),
        "U" => Some(Key::U),
        "V" => Some(Key::V),
        "W" => Some(Key::W),
        "X" => Some(Key::X),
        "Y" => Some(Key::Y),
        "Z" => Some(Key::Z),
        _ => None,
    }
}

pub fn is_shortcut_pressed(ctx: &egui::Context, bindings: &[String]) -> bool {
    bindings.iter().any(|name| {
        key_from_name(name)
            .map(|key| ctx.input(|i| i.key_pressed(key)))
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_from_name_roundtrip() {
        assert_eq!(key_from_name("Space"), Some(Key::Space));
        assert_eq!(key_from_name("ArrowRight"), Some(Key::ArrowRight));
        assert_eq!(key_from_name("UnknownKey"), None);
    }

    #[test]
    fn test_key_from_name_known_keys() {
        assert_eq!(key_from_name("Escape"), Some(Key::Escape));
        assert_eq!(key_from_name("F11"), Some(Key::F11));
    }
}
