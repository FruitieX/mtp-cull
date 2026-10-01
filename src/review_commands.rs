use eframe::egui;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Bursts,
    NextBurst,
    PreviousBurst,
    FilterUnreviewed,
    FilterKeep,
    FilterReject,
    FilterAll,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
    CenterRegion,
    RegionLeft,
    RegionRight,
    RegionUp,
    RegionDown,
    RegionGrow,
    RegionShrink,
    ThresholdUp,
    ThresholdDown,
    OpacityUp,
    OpacityDown,
    AlignLeft,
    AlignRight,
    AlignUp,
    AlignDown,
    Close,
    RawFolder,
    Open,
    Camera,
    Previous,
    Next,
    RowUp,
    RowDown,
    NextUnreviewed,
    Reject,
    Keep,
    Clear,
    ToggleKeep,
    Zoom,
    Pin,
    Compare,
    Blink,
    Swap,
    ActivePane,
    Peaking,
    Region,
    ClearRegion,
    DividerLeft,
    DividerRight,
    DividerCenter,
    ResetAlignment,
    Undo,
    Redo,
    Jpeg,
    Raw,
    Video,
    All,
    BulkKeep,
    BulkReject,
    SelectAll,
    DeselectAll,
    ToggleReelSelection,
    ReelMode,
    ReelPosition,
    Import,
    Presets,
    Settings,
    Help,
    Palette,
    Retry,
    Pause,
    Cancel,
}
#[derive(Clone, Copy)]
pub struct Spec {
    pub command: Command,
    pub id: &'static str,
    pub label: &'static str,
    pub key: &'static str,
}
pub const COMMANDS: &[Spec] = &[
    Spec {
        command: Command::Bursts,
        id: "bursts",
        label: "Toggle burst grouping",
        key: "Ctrl+B",
    },
    Spec {
        command: Command::NextBurst,
        id: "next_burst",
        label: "Next burst",
        key: "Ctrl+PageDown",
    },
    Spec {
        command: Command::PreviousBurst,
        id: "previous_burst",
        label: "Previous burst",
        key: "Ctrl+PageUp",
    },
    Spec {
        command: Command::FilterUnreviewed,
        id: "filter_unreviewed",
        label: "Show unreviewed",
        key: "U",
    },
    Spec {
        command: Command::FilterKeep,
        id: "filter_keep",
        label: "Show keepers",
        key: "K",
    },
    Spec {
        command: Command::FilterReject,
        id: "filter_reject",
        label: "Show rejects",
        key: "X",
    },
    Spec {
        command: Command::FilterAll,
        id: "decision_filter_all",
        label: "Show all decisions",
        key: "Shift+A",
    },
    Spec {
        command: Command::PanLeft,
        id: "pan_left",
        label: "Pan left",
        key: "Ctrl+ArrowLeft",
    },
    Spec {
        command: Command::PanRight,
        id: "pan_right",
        label: "Pan right",
        key: "Ctrl+ArrowRight",
    },
    Spec {
        command: Command::PanUp,
        id: "pan_up",
        label: "Pan up",
        key: "Ctrl+ArrowUp",
    },
    Spec {
        command: Command::PanDown,
        id: "pan_down",
        label: "Pan down",
        key: "Ctrl+ArrowDown",
    },
    Spec {
        command: Command::CenterRegion,
        id: "center_region",
        label: "Create centered comparison region",
        key: "Shift+R",
    },
    Spec {
        command: Command::RegionLeft,
        id: "region_left",
        label: "Move region left",
        key: "Shift+ArrowLeft",
    },
    Spec {
        command: Command::RegionRight,
        id: "region_right",
        label: "Move region right",
        key: "Shift+ArrowRight",
    },
    Spec {
        command: Command::RegionUp,
        id: "region_up",
        label: "Move region up",
        key: "Shift+ArrowUp",
    },
    Spec {
        command: Command::RegionDown,
        id: "region_down",
        label: "Move region down",
        key: "Shift+ArrowDown",
    },
    Spec {
        command: Command::RegionGrow,
        id: "region_grow",
        label: "Grow comparison region",
        key: "Ctrl+Equals",
    },
    Spec {
        command: Command::RegionShrink,
        id: "region_shrink",
        label: "Shrink comparison region",
        key: "Ctrl+Minus",
    },
    Spec {
        command: Command::ThresholdUp,
        id: "threshold_up",
        label: "Raise focus threshold",
        key: "Equals",
    },
    Spec {
        command: Command::ThresholdDown,
        id: "threshold_down",
        label: "Lower focus threshold",
        key: "Minus",
    },
    Spec {
        command: Command::OpacityUp,
        id: "opacity_up",
        label: "Increase focus opacity",
        key: "Shift+Equals",
    },
    Spec {
        command: Command::OpacityDown,
        id: "opacity_down",
        label: "Decrease focus opacity",
        key: "Shift+Minus",
    },
    Spec {
        command: Command::AlignLeft,
        id: "align_left",
        label: "Align B left",
        key: "Ctrl+Alt+ArrowLeft",
    },
    Spec {
        command: Command::AlignRight,
        id: "align_right",
        label: "Align B right",
        key: "Ctrl+Alt+ArrowRight",
    },
    Spec {
        command: Command::AlignUp,
        id: "align_up",
        label: "Align B up",
        key: "Ctrl+Alt+ArrowUp",
    },
    Spec {
        command: Command::AlignDown,
        id: "align_down",
        label: "Align B down",
        key: "Ctrl+Alt+ArrowDown",
    },
    Spec {
        command: Command::Close,
        id: "close",
        label: "Close review session",
        key: "Ctrl+W",
    },
    Spec {
        command: Command::RawFolder,
        id: "raw_folder",
        label: "Choose companion RAW folder",
        key: "Ctrl+Shift+O",
    },
    Spec {
        command: Command::Open,
        id: "open",
        label: "Open local folder",
        key: "Ctrl+O",
    },
    Spec {
        command: Command::Camera,
        id: "camera",
        label: "Open camera",
        key: "Ctrl+M",
    },
    Spec {
        command: Command::Previous,
        id: "previous",
        label: "Previous candidate",
        key: "ArrowLeft",
    },
    Spec {
        command: Command::Next,
        id: "next",
        label: "Next candidate",
        key: "ArrowRight",
    },
    Spec {
        command: Command::RowUp,
        id: "row_up",
        label: "Previous grid row / previous candidate",
        key: "ArrowUp",
    },
    Spec {
        command: Command::RowDown,
        id: "row_down",
        label: "Next grid row / next candidate",
        key: "ArrowDown",
    },
    Spec {
        command: Command::NextUnreviewed,
        id: "next_unreviewed",
        label: "Next unreviewed",
        key: "N",
    },
    Spec {
        command: Command::Reject,
        id: "reject",
        label: "Reject selected images / active pane",
        key: "2",
    },
    Spec {
        command: Command::Keep,
        id: "keep",
        label: "Keep selected images / active pane",
        key: "1",
    },
    Spec {
        command: Command::Clear,
        id: "clear",
        label: "Mark selected images unreviewed / active pane",
        key: "0",
    },
    Spec {
        command: Command::ToggleKeep,
        id: "toggle_keep",
        label: "Toggle Keep for selected images / active pane",
        key: "Space",
    },
    Spec {
        command: Command::Zoom,
        id: "zoom",
        label: "Fit / 100% zoom",
        key: "Z",
    },
    Spec {
        command: Command::Pin,
        id: "pin",
        label: "Pin candidate as A",
        key: "P",
    },
    Spec {
        command: Command::Compare,
        id: "compare",
        label: "Cycle comparison view",
        key: "C",
    },
    Spec {
        command: Command::Blink,
        id: "blink",
        label: "Hold to view A",
        key: "B",
    },
    Spec {
        command: Command::Swap,
        id: "swap",
        label: "Swap A and B",
        key: "S",
    },
    Spec {
        command: Command::ActivePane,
        id: "active_pane",
        label: "Switch decision pane",
        key: "Tab",
    },
    Spec {
        command: Command::Peaking,
        id: "peaking",
        label: "Toggle focus overlay",
        key: "H",
    },
    Spec {
        command: Command::Region,
        id: "region",
        label: "Draw shared focus region",
        key: "R",
    },
    Spec {
        command: Command::ClearRegion,
        id: "clear_region",
        label: "Clear focus region",
        key: "Ctrl+Shift+R",
    },
    Spec {
        command: Command::DividerLeft,
        id: "divider_left",
        label: "Move wipe divider left",
        key: "[",
    },
    Spec {
        command: Command::DividerRight,
        id: "divider_right",
        label: "Move wipe divider right",
        key: "]",
    },
    Spec {
        command: Command::DividerCenter,
        id: "divider_center",
        label: "Center wipe divider",
        key: "\\",
    },
    Spec {
        command: Command::ResetAlignment,
        id: "reset_alignment",
        label: "Reset B alignment",
        key: "Alt+R",
    },
    Spec {
        command: Command::Undo,
        id: "undo",
        label: "Undo decision",
        key: "Ctrl+Z",
    },
    Spec {
        command: Command::Redo,
        id: "redo",
        label: "Redo decision",
        key: "Ctrl+Shift+Z",
    },
    Spec {
        command: Command::Jpeg,
        id: "filter_jpeg",
        label: "Show JPEG",
        key: "J",
    },
    Spec {
        command: Command::Raw,
        id: "filter_raw",
        label: "Show RAW",
        key: "F",
    },
    Spec {
        command: Command::Video,
        id: "filter_video",
        label: "Show videos",
        key: "V",
    },
    Spec {
        command: Command::All,
        id: "filter_all",
        label: "Show all media",
        key: "A",
    },
    Spec {
        command: Command::BulkKeep,
        id: "bulk_keep",
        label: "Keep visible images",
        key: "Ctrl+Shift+K",
    },
    Spec {
        command: Command::BulkReject,
        id: "bulk_reject",
        label: "Reject visible images",
        key: "Ctrl+Backspace",
    },
    Spec {
        command: Command::SelectAll,
        id: "select_all",
        label: "Select all visible images",
        key: "Ctrl+A",
    },
    Spec {
        command: Command::DeselectAll,
        id: "deselect_all",
        label: "Clear reel selection",
        key: "Ctrl+Shift+A",
    },
    Spec {
        command: Command::ToggleReelSelection,
        id: "toggle_reel_selection",
        label: "Toggle active photo in reel selection",
        key: "Insert",
    },
    Spec {
        command: Command::ReelMode,
        id: "reel_mode",
        label: "Toggle reel strip / grid",
        key: "Ctrl+G",
    },
    Spec {
        command: Command::ReelPosition,
        id: "reel_position",
        label: "Move reel: Bottom / Left / Right",
        key: "Ctrl+Shift+G",
    },
    Spec {
        command: Command::Import,
        id: "import",
        label: "Review import",
        key: "Ctrl+I",
    },
    Spec {
        command: Command::Presets,
        id: "import_presets",
        label: "Manage import presets",
        key: "Ctrl+Shift+I",
    },
    Spec {
        command: Command::Settings,
        id: "settings",
        label: "Settings and shortcuts",
        key: "Ctrl+Comma",
    },
    Spec {
        command: Command::Help,
        id: "help",
        label: "Keyboard help",
        key: "F1",
    },
    Spec {
        command: Command::Palette,
        id: "palette",
        label: "Command palette",
        key: "Ctrl+P",
    },
    Spec {
        command: Command::Retry,
        id: "retry",
        label: "Retry previews",
        key: "F5",
    },
    Spec {
        command: Command::Pause,
        id: "pause",
        label: "Pause / resume camera staging",
        key: "Ctrl+Space",
    },
    Spec {
        command: Command::Cancel,
        id: "cancel",
        label: "Cancel current operation",
        key: "Escape",
    },
];
pub fn parse(text: &str) -> Option<egui::KeyboardShortcut> {
    let mut modifiers = egui::Modifiers::NONE;
    let mut key = None;
    for part in text.split('+') {
        match part.trim() {
            "Ctrl" => modifiers.ctrl = true,
            "Shift" => modifiers.shift = true,
            "Alt" => modifiers.alt = true,
            "Cmd" => modifiers.command = true,
            name => {
                if key.is_some() {
                    return None;
                }
                key = egui::Key::from_name(name);
                key?;
            }
        }
    }
    Some(egui::KeyboardShortcut::new(modifiers, key?))
}
pub fn validate(bindings: &std::collections::BTreeMap<String, String>) -> Result<(), String> {
    let mut assigned = Vec::new();
    for spec in COMMANDS {
        let text = bindings.get(spec.id).map_or(spec.key, String::as_str);
        let shortcut =
            parse(text).ok_or_else(|| format!("Invalid shortcut for {}: {text}", spec.label))?;
        let mut shortcut = shortcut;
        if !cfg!(target_os = "macos") && shortcut.modifiers.command {
            shortcut.modifiers.ctrl = true;
            shortcut.modifiers.command = false;
        }
        if let Some((_, label)) = assigned.iter().find(|(other, _)| *other == shortcut) {
            return Err(format!("{} and {} both use {text}", spec.label, label));
        }
        assigned.push((shortcut, spec.label));
    }
    Ok(())
}
/// Exact modifiers prevent Shift+commands from triggering their plain variants.
pub fn consume(events: &mut Vec<egui::Event>, shortcut: &egui::KeyboardShortcut) -> bool {
    let mut found = false;
    events.retain(|event| {
        let matches=matches!(event,egui::Event::Key{key,modifiers,pressed:true,..} if *key==shortcut.logical_key && modifiers.matches_exact(shortcut.modifiers));
        found|=matches;!matches
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_defaults_have_unique_ids_and_independent_all_filters() {
        let bindings = COMMANDS
            .iter()
            .map(|spec| (spec.id.to_owned(), spec.key.to_owned()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            bindings.len(),
            COMMANDS.len(),
            "command IDs must be unique for persistence"
        );
        validate(&bindings).unwrap();
        assert_eq!(bindings["filter_all"], "A");
        assert_eq!(bindings["decision_filter_all"], "Shift+A");
    }
    #[test]
    fn shifted_commands_do_not_trigger_plain_commands() {
        let mut events = vec![egui::Event::Key {
            key: egui::Key::Z,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        }];
        assert!(!consume(&mut events, &parse("Ctrl+Z").unwrap()));
        assert!(consume(&mut events, &parse("Ctrl+Shift+Z").unwrap()));
        assert!(events.is_empty());
    }
    #[test]
    fn defaults_are_valid_and_conflicting_customizations_are_rejected() {
        let mut bindings = std::collections::BTreeMap::new();
        validate(&bindings).unwrap();
        bindings.insert("keep".into(), "2".into());
        assert!(validate(&bindings).is_err());
        assert!(parse("Ctrl+made-up").is_none());
    }
}
