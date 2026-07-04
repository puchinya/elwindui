/// Whether a hot-reloaded `view` update requires a full remount (state reset) or can be
/// applied as a differential patch (state preserved). See docs/elwindui_spec.md 付録B.4.
#[derive(Debug, PartialEq, Eq)]
pub enum ReloadAction {
    Remount,
    Patch,
}

/// A `#[param]` field change always forces a remount; if only `prop` fields changed,
/// a differential patch is enough.
pub fn decide_reload_action(any_param_field_changed: bool) -> ReloadAction {
    if any_param_field_changed {
        ReloadAction::Remount
    } else {
        ReloadAction::Patch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_change_forces_remount() {
        assert_eq!(decide_reload_action(true), ReloadAction::Remount);
    }

    #[test]
    fn prop_only_change_patches() {
        assert_eq!(decide_reload_action(false), ReloadAction::Patch);
    }
}
