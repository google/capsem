use tray_icon::Icon;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    /// No VMs running (grey icon)
    Idle,
    /// VMs running (green icon)
    Active,
    /// Gateway unreachable (red icon)
    Error,
}

/// Load a tray icon for the given state.
///
/// Uses programmatic solid-color icons as placeholders until real PNG assets
/// are created in SS6. macOS template images need proper design work.
pub fn load_icon(state: TrayState) -> Icon {
    let (r, g, b) = match state {
        TrayState::Idle => (128, 128, 128),   // grey
        TrayState::Active => (76, 175, 80),   // green
        TrayState::Error => (244, 67, 54),    // red
    };

    // 22x22 solid color icon (macOS menu bar standard size)
    let size = 22u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for _ in 0..(size * size) {
        rgba.push(r);
        rgba.push(g);
        rgba.push(b);
        rgba.push(255);
    }

    Icon::from_rgba(rgba, size, size).expect("failed to create icon from RGBA data")
}
