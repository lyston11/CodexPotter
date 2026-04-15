pub fn effective_yolo_enabled(cli_override: bool) -> bool {
    if cli_override {
        return true;
    }

    codex_tui::load_potter_yolo_enabled().unwrap_or_default()
}
