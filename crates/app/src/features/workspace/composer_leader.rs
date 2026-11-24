use super::*;
use crate::features::shortcuts::keystroke_string;
use gentle_core::{
    keyboard::format_binding,
    shortcuts::{label, same_keys},
};

/// Owns the sequence independently of GPUI's timed pending input.
pub(super) struct ComposerLeader {
    input: Vec<Keystroke>,
    context: Vec<KeyContext>,
}

enum Choice {
    Keep,
    Cancel,
    Dispatch(Box<dyn Action>),
}

impl ComposerLeader {
    fn start(
        key: &Keystroke,
        leader: &str,
        context: Vec<KeyContext>,
        map: &Keymap,
    ) -> Option<Self> {
        if !same_keys(&keystroke_string(key), leader) {
            return None;
        }
        let input = vec![key.clone()];
        let (exact, pending) = map.bindings_for_input(&input, &context);
        (exact.is_empty() && pending).then_some(Self { input, context })
    }

    fn choose(&mut self, key: &Keystroke, map: &Keymap) -> Choice {
        if keystroke_string(key) == "escape" {
            return Choice::Cancel;
        }
        self.input.push(key.clone());
        let (bindings, pending) = map.bindings_for_input(&self.input, &self.context);
        if pending {
            return Choice::Keep;
        }
        if let Some(binding) = bindings.first() {
            return Choice::Dispatch(binding.action().boxed_clone());
        }
        self.input.pop();
        Choice::Keep
    }

    pub(super) fn hint(&self, cx: &App) -> Option<WhichKey> {
        let macos = cfg!(target_os = "macos");
        let mut entries = Vec::new();
        for binding in cx
            .key_bindings()
            .borrow()
            .possible_next_bindings_for_input(&self.input, &self.context)
        {
            if let Some(label) = label(binding.action().name()) {
                let key = keystroke_string(binding.keystrokes()[self.input.len()].inner());
                let entry = (format_binding(&key, macos), label);
                if !entries.contains(&entry) {
                    entries.push(entry);
                }
            }
        }
        (!entries.is_empty()).then(|| WhichKey {
            title: format_binding(
                &self
                    .input
                    .iter()
                    .map(keystroke_string)
                    .collect::<Vec<_>>()
                    .join(" "),
                macos,
            ),
            entries,
            footer: "Esc cancels",
        })
    }
}

impl Workspace {
    pub(super) fn cancel_composer_leader(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((origin, _)) = self.composer_leader.take() {
            if self.pane_mode_focus.is_focused(window) {
                origin.focus(window, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn composer_leader_key(
        &mut self,
        key: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pane_mode_focus.is_focused(window)
            && let Some((_, selection)) = self.composer_leader.as_mut()
        {
            let choice = selection.choose(key, &cx.key_bindings().borrow());
            match choice {
                Choice::Keep => cx.notify(),
                Choice::Cancel => self.cancel_composer_leader(window, cx),
                Choice::Dispatch(action) => {
                    // Restore the original action scope before dispatch; an action may move focus elsewhere.
                    self.cancel_composer_leader(window, cx);
                    window.dispatch_action(action, cx);
                }
            }
            cx.stop_propagation();
            return;
        }
        if self.pane_mode || self.overlay.is_some() || window.has_pending_keystrokes() {
            return;
        }
        let Some(selection) = ComposerLeader::start(
            key,
            &crate::app::leader(cx),
            window.context_stack(),
            &cx.key_bindings().borrow(),
        ) else {
            return;
        };
        if self
            .sessions()
            .iter()
            .any(|session| session.read(cx).composer_has_focus(window, cx))
            && let Some(origin) = window.focused(cx)
        {
            self.composer_leader = Some((origin, selection));
            self.pane_mode_focus.focus(window, cx);
            cx.notify();
            cx.stop_propagation();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Choice, ComposerLeader};
    use gpui_kit::{KeyBinding, Keymap, Keystroke};

    #[gpui::test]
    fn composer_leader_ignores_elapsed_timeout_and_resolves_configured_choice(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let map = Keymap::new(vec![KeyBinding::new(
            "ctrl-a f",
            crate::features::workspace::FocusFiles,
            None,
        )]);
        let prefix = Keystroke::parse("ctrl-a").unwrap();
        assert!(ComposerLeader::start(&prefix, "ctrl-space", vec![], &map).is_none());
        let mut selection = ComposerLeader::start(&prefix, "ctrl-a", vec![], &map).unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(3));
        assert!(matches!(
            selection.choose(&Keystroke::parse("z").unwrap(), &map),
            Choice::Keep
        ));
        let Choice::Dispatch(action) = selection.choose(&Keystroke::parse("f").unwrap(), &map)
        else {
            panic!("delayed choice must dispatch");
        };
        assert!(
            action
                .as_any()
                .is::<crate::features::workspace::FocusFiles>()
        );
        let mut selection = ComposerLeader::start(&prefix, "ctrl-a", vec![], &map).unwrap();
        assert!(matches!(
            selection.choose(&Keystroke::parse("escape").unwrap(), &map),
            Choice::Cancel
        ));
    }
}
