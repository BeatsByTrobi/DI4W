//! Dark (black pill, the default) or light island. The settings app draws its preview with it too.

pub const DARK: &str = "";
pub const LIGHT: &str = "Light";
pub const SYSTEM: &str = "System";

/// True when the setting means a light island right now ("System" follows Windows' taskbar color).
pub fn is_light(setting: &str) -> bool {
    match setting {
        LIGHT => true,
        SYSTEM => windows_uses_light_theme(),
        _ => false,
    }
}

/// Windows' own light/dark choice for the taskbar and Start ("Choose your default Windows mode").
pub fn windows_uses_light_theme() -> bool {
    crate::registry::read_dword(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "SystemUsesLightTheme")
        .is_some_and(|v| v != 0)
}

/// Windows' light/dark choice for apps ("Choose your default app mode"), for the settings app's own theme.
pub fn apps_use_light_theme() -> bool {
    crate::registry::read_dword(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "AppsUseLightTheme")
        .is_none_or(|v| v != 0)
}
