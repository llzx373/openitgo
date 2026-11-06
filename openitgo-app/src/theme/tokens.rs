#![allow(dead_code)]

use egui::Color32;

#[derive(Debug, Clone, Copy)]
pub struct FluentPalette {
    pub canvas: Color32,
    pub surface: Color32,
    pub surface_alt: Color32,
    pub card: Color32,
    pub card_hover: Color32,
    pub control: Color32,
    pub control_hover: Color32,
    pub control_pressed: Color32,
    pub stroke: Color32,
    pub stroke_strong: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_pressed: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
}

impl FluentPalette {
    pub const fn light() -> Self {
        Self {
            canvas: Color32::from_rgb(0xF3, 0xF3, 0xF3),
            surface: Color32::from_rgb(0xF9, 0xF9, 0xF9),
            surface_alt: Color32::from_rgb(0xEF, 0xEF, 0xEF),
            card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            card_hover: Color32::from_rgb(0xF5, 0xF5, 0xF5),
            control: Color32::from_rgb(0xFD, 0xFD, 0xFD),
            control_hover: Color32::from_rgb(0xF5, 0xF5, 0xF5),
            control_pressed: Color32::from_rgb(0xE9, 0xE9, 0xE9),
            stroke: Color32::from_rgb(0xE0, 0xE0, 0xE0),
            stroke_strong: Color32::from_rgb(0xC7, 0xC7, 0xC7),
            text: Color32::from_rgb(0x1B, 0x1B, 0x1B),
            text_secondary: Color32::from_rgb(0x61, 0x61, 0x61),
            accent: Color32::from_rgb(0x0F, 0x6C, 0xBD),
            accent_hover: Color32::from_rgb(0x11, 0x5E, 0xA3),
            accent_pressed: Color32::from_rgb(0x0C, 0x4A, 0x80),
            on_accent: Color32::WHITE,
            danger: Color32::from_rgb(0xC4, 0x2B, 0x1C),
        }
    }

    pub const fn dark() -> Self {
        Self {
            canvas: Color32::from_rgb(0x20, 0x20, 0x20),
            surface: Color32::from_rgb(0x28, 0x28, 0x28),
            surface_alt: Color32::from_rgb(0x2C, 0x2C, 0x2C),
            card: Color32::from_rgb(0x2B, 0x2B, 0x2B),
            card_hover: Color32::from_rgb(0x32, 0x32, 0x32),
            control: Color32::from_rgb(0x33, 0x33, 0x33),
            control_hover: Color32::from_rgb(0x3A, 0x3A, 0x3A),
            control_pressed: Color32::from_rgb(0x2E, 0x2E, 0x2E),
            stroke: Color32::from_rgb(0x45, 0x45, 0x45),
            stroke_strong: Color32::from_rgb(0x5C, 0x5C, 0x5C),
            text: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            text_secondary: Color32::from_rgb(0xC5, 0xC5, 0xC5),
            accent: Color32::from_rgb(0x47, 0x9E, 0xF5),
            accent_hover: Color32::from_rgb(0x62, 0xAE, 0xF7),
            accent_pressed: Color32::from_rgb(0x35, 0x88, 0xD4),
            on_accent: Color32::from_rgb(0x10, 0x10, 0x10),
            danger: Color32::from_rgb(0xFF, 0x99, 0xA4),
        }
    }
}

pub mod spacing {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 32.0;
}

pub mod size {
    pub const CONTROL_COMPACT: f32 = 28.0;
    pub const CONTROL: f32 = 32.0;
    pub const SETTINGS_NAV_WIDTH: f32 = 208.0;
    pub const SETTINGS_CONTENT_MAX: f32 = 1040.0;
}

pub mod radius {
    pub const CONTROL: u8 = 4;
    pub const CARD: u8 = 8;
}

pub mod text {
    pub const SMALL: f32 = 12.0;
    pub const BODY: f32 = 14.0;
    pub const SUBTITLE: f32 = 16.0;
    pub const TITLE: f32 = 24.0;
}
