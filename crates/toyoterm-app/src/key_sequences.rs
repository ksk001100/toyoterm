use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use toyoterm_api::{NativeAction, PaneId};
use toyoterm_script::{KeymapScope, ScriptSnapshot, TransientKeymap};
use winit::keyboard::PhysicalKey;

#[derive(Debug, PartialEq)]
pub(super) enum SequenceDispatch {
    Unassigned,
    Consumed,
    Binding(String),
}

#[derive(Debug, PartialEq)]
pub(super) enum TransientDispatch {
    Inactive,
    Consumed,
    Invoke { id: u64, key: Option<String> },
}

pub(super) fn native_action_enabled(action: &NativeAction, visual_mode: bool) -> bool {
    visual_mode
        || !matches!(
            action,
            NativeAction::EndVisualSelection
                | NativeAction::SelectVisualSelection
                | NativeAction::MoveVisualSelection(_)
                | NativeAction::YankSelection
        )
}

struct PendingSequence {
    prefix: String,
    pane: PaneId,
    deadline: Option<Instant>,
}

fn modifier_only(candidates: &[String]) -> bool {
    !candidates.is_empty()
        && candidates.iter().all(|key| {
            key.rsplit_once("PHYSICAL:").is_some_and(|(_, code)| {
                matches!(
                    code,
                    "CONTROLLEFT"
                        | "CONTROLRIGHT"
                        | "SHIFTLEFT"
                        | "SHIFTRIGHT"
                        | "ALTLEFT"
                        | "ALTRIGHT"
                        | "SUPERLEFT"
                        | "SUPERRIGHT"
                )
            })
        })
}

/// Native prefix lookup. The registry is rebuilt only when a script snapshot
/// changes, so ordinary PTY input never scans Ruby registrations or calls Ruby.
#[derive(Default)]
pub(super) struct KeySequences {
    bindings: HashSet<String>,
    prefixes: HashSet<String>,
    single_bindings: HashMap<String, bool>,
    timeout: Option<Duration>,
    pending: Option<PendingSequence>,
    held: HashSet<PhysicalKey>,
    pane: Option<PaneId>,
    priorities: HashMap<String, u64>,
    transients: Vec<TransientKeymap>,
    retired_transients: HashSet<u64>,
    transient_prefix: Option<(u64, PendingSequence)>,
}

impl KeySequences {
    pub(super) fn new(snapshot: &ScriptSnapshot) -> Self {
        Self::for_pane(snapshot, None)
    }

    fn for_pane(snapshot: &ScriptSnapshot, pane: Option<PaneId>) -> Self {
        let effective = pane
            .and_then(|pane| snapshot.keymap_bindings_for(pane))
            .or_else(|| snapshot.keymap_bindings.get(&KeymapScope::Global));
        let native_actions = effective.map_or(&snapshot.native_actions, |bindings| {
            &bindings.native_actions
        });
        let keybindings = effective.map_or(&snapshot.keybindings, |bindings| &bindings.keybindings);
        let bindings: HashSet<String> = keybindings
            .iter()
            .chain(native_actions.keys())
            .filter(|key| key.contains(' '))
            .cloned()
            .collect();
        let mut prefixes = HashSet::new();
        let mut single_bindings: HashMap<String, bool> = keybindings
            .iter()
            .filter(|key| !key.contains(' '))
            .map(|key| (key.clone(), true))
            .collect();
        for (key, action) in native_actions {
            if !key.contains(' ') {
                single_bindings.insert(key.clone(), native_action_enabled(action, false));
            }
        }
        let mut priorities =
            effective.map_or_else(HashMap::new, |bindings| bindings.priorities.clone());
        for binding in &bindings {
            let rank = priorities.get(binding).copied().unwrap_or(0);
            for (index, _) in binding.match_indices(' ') {
                let prefix = binding[..index].to_owned();
                prefixes.insert(prefix.clone());
                priorities
                    .entry(prefix)
                    .and_modify(|priority| *priority = (*priority).max(rank))
                    .or_insert(rank);
            }
        }
        Self {
            bindings,
            prefixes,
            single_bindings,
            timeout: snapshot
                .config
                .key_sequence_timeout_ms
                .map(Duration::from_millis),
            pending: None,
            held: HashSet::new(),
            pane,
            priorities,
            transients: effective.map_or_else(Vec::new, |bindings| bindings.transients.clone()),
            retired_transients: HashSet::new(),
            transient_prefix: None,
        }
    }

    pub(super) fn clear(&mut self) {
        self.pending = None;
        self.transient_prefix = None;
    }

    pub(super) fn replace_registry(&mut self, snapshot: &ScriptSnapshot) {
        let mut replacement = Self::for_pane(snapshot, self.pane);
        replacement.held = std::mem::take(&mut self.held);
        // Earlier completions may still contain a map already consumed by a
        // later key press. Keep it retired until the VM acknowledges removal.
        replacement.retired_transients = std::mem::take(&mut self.retired_transients);
        replacement.retired_transients.retain(|id| {
            snapshot
                .keymap_bindings
                .values()
                .any(|bindings| bindings.transients.iter().any(|map| map.id == *id))
        });
        *self = replacement;
    }

    pub(super) fn select_pane(&mut self, snapshot: &ScriptSnapshot, pane: PaneId) {
        if self.pane != Some(pane) {
            let mut replacement = Self::for_pane(snapshot, Some(pane));
            replacement.held = std::mem::take(&mut self.held);
            replacement.retired_transients = std::mem::take(&mut self.retired_transients);
            *self = replacement;
        }
    }

    pub(super) fn release(&mut self, key: PhysicalKey) {
        self.held.remove(&key);
    }

    pub(super) fn focus_lost(&mut self) {
        self.clear();
        self.held.clear();
    }

    pub(super) fn hold(&mut self, key: PhysicalKey) {
        self.held.insert(key);
    }

    pub(super) fn is_held(&self, key: PhysicalKey) -> bool {
        self.held.contains(&key)
    }

    pub(super) fn reset_transients(&mut self) {
        self.transients.clear();
        self.retired_transients.clear();
        self.transient_prefix = None;
    }

    pub(super) fn advance_transient(
        &mut self,
        candidates: &[String],
        pane: PaneId,
        repeat: bool,
        now: Instant,
    ) -> TransientDispatch {
        let Some(map) = self
            .transients
            .iter()
            .find(|map| !self.retired_transients.contains(&map.id))
        else {
            self.transient_prefix = None;
            return TransientDispatch::Inactive;
        };
        if repeat {
            return TransientDispatch::Consumed;
        }
        let id = map.id;
        let pending = self.transient_prefix.take().filter(|(owner, pending)| {
            *owner == id
                && pending.pane == pane
                && pending.deadline.is_none_or(|deadline| now <= deadline)
        });
        let mut key = None;
        let cancel = candidates
            .iter()
            .any(|key| key == "ESCAPE" || key == "CTRL+G");
        if !cancel {
            for candidate in candidates {
                let candidate = pending.as_ref().map_or_else(
                    || candidate.clone(),
                    |(_, pending)| format!("{} {candidate}", pending.prefix),
                );
                if map.keys.contains(&candidate) {
                    key = Some(candidate);
                    break;
                }
                let prefix = format!("{candidate} ");
                if map.keys.iter().any(|key| key.starts_with(&prefix)) {
                    self.transient_prefix = Some((
                        id,
                        PendingSequence {
                            prefix: candidate,
                            pane,
                            deadline: self.timeout.and_then(|timeout| now.checked_add(timeout)),
                        },
                    ));
                    return TransientDispatch::Consumed;
                }
            }
        }
        // Modifier presses between chords are not commands. Explicit physical
        // modifier bindings still win above; otherwise preserve the prefix.
        if key.is_none() && (modifier_only(candidates) || candidates.is_empty()) {
            self.transient_prefix = pending;
            return TransientDispatch::Consumed;
        }
        if key.is_none() || map.once {
            self.retired_transients.insert(id);
        }
        self.pending = None;
        TransientDispatch::Invoke { id, key }
    }

    pub(super) fn has_layer_binding(&self, candidates: &[String], visual_mode: bool) -> bool {
        candidates.iter().any(|key| {
            self.priorities.get(key).is_some_and(|rank| *rank > 0)
                && self
                    .single_bindings
                    .get(key)
                    .is_some_and(|always| *always || visual_mode)
        })
    }

    pub(super) fn advance(
        &mut self,
        candidates: &[String],
        pane: PaneId,
        repeat: bool,
        visual_mode: bool,
        now: Instant,
    ) -> SequenceDispatch {
        let modifiers = modifier_only(candidates);
        let pending = self.pending.take().filter(|pending| {
            pending.pane == pane && pending.deadline.is_none_or(|deadline| now <= deadline)
        });
        let mut candidates: Vec<&String> = candidates.iter().collect();
        candidates.sort_by_key(|key| {
            let key = pending.as_ref().map_or_else(
                || (*key).clone(),
                |pending| format!("{} {key}", pending.prefix),
            );
            std::cmp::Reverse(self.priorities.get(&key).copied().unwrap_or(0))
        });
        if let Some(pending) = pending {
            // Repeats cannot finish a sequence or extend its timeout. Cancellation
            // is evaluated after this check so repeating a prefix remains harmless.
            if repeat {
                self.pending = Some(pending);
                return SequenceDispatch::Consumed;
            }
            if candidates
                .iter()
                .any(|key| key.as_str() == "ESCAPE" || key.as_str() == "CTRL+G")
            {
                return SequenceDispatch::Consumed;
            }
            for candidate in &candidates {
                let key = format!("{} {candidate}", pending.prefix);
                if self.bindings.contains(&key) {
                    return SequenceDispatch::Binding(key);
                }
                if self.prefixes.contains(&key) {
                    self.pending = Some(PendingSequence {
                        prefix: key,
                        pane,
                        deadline: self.timeout.and_then(|timeout| now.checked_add(timeout)),
                    });
                    return SequenceDispatch::Consumed;
                }
            }
            // Once a prefix is accepted, an invalid suffix cancels the sequence
            // and is consumed. Command keystrokes must not leak into the shell.
            if modifiers {
                self.pending = Some(pending);
            }
            return SequenceDispatch::Consumed;
        }
        for candidate in candidates {
            if self
                .single_bindings
                .get(candidate)
                .is_some_and(|always| *always || visual_mode)
            {
                return SequenceDispatch::Unassigned;
            }
            if self.prefixes.contains(candidate) {
                if !repeat {
                    self.pending = Some(PendingSequence {
                        prefix: candidate.clone(),
                        pane,
                        deadline: self.timeout.and_then(|timeout| now.checked_add(timeout)),
                    });
                }
                return SequenceDispatch::Consumed;
            }
        }
        SequenceDispatch::Unassigned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use toyoterm_api::NativeAction;
    use toyoterm_config::ToyotermConfig;

    fn snapshot() -> ScriptSnapshot {
        ScriptSnapshot {
            config: ToyotermConfig::default(),
            color_presets: HashMap::new(),
            native_actions: HashMap::from([
                ("CTRL+X CTRL+F".into(), NativeAction::NewTab),
                ("CTRL+X R B".into(), NativeAction::ToggleZoom),
            ]),
            keybindings: HashSet::from(["CTRL+X PHYSICAL:KEYH".into()]),
            event_names: HashSet::new(),
            user_command_names: HashSet::new(),
            keymap_bindings: HashMap::new(),
            timers: Vec::new(),
            advised_actions: HashSet::new(),
        }
    }

    fn press(keys: &mut KeySequences, key: &str, now: Instant) -> SequenceDispatch {
        keys.advance(&[key.to_owned()], PaneId(1), false, false, now)
    }

    fn transient_snapshot(once: bool) -> ScriptSnapshot {
        let mut snapshot = snapshot();
        snapshot.keymap_bindings.insert(
            KeymapScope::Global,
            toyoterm_script::KeymapBindings {
                transients: vec![TransientKeymap {
                    id: 10,
                    once,
                    keys: HashSet::from(["S".into(), "CTRL+X B".into(), "PHYSICAL:KEYH".into()]),
                }],
                ..Default::default()
            },
        );
        snapshot
    }

    fn transient_press(keys: &mut KeySequences, key: &str, now: Instant) -> TransientDispatch {
        keys.advance_transient(&[key.to_owned()], PaneId(1), false, now)
    }

    #[test]
    fn transient_one_shots_retire_immediately_and_ignore_old_snapshots_and_pane_switches() {
        let snapshot = transient_snapshot(true);
        let mut keys = KeySequences::new(&snapshot);
        let now = Instant::now();
        assert_eq!(
            transient_press(&mut keys, "S", now),
            TransientDispatch::Invoke {
                id: 10,
                key: Some("S".into())
            }
        );
        assert_eq!(
            transient_press(&mut keys, "S", now),
            TransientDispatch::Inactive
        );
        keys.replace_registry(&snapshot);
        keys.select_pane(&snapshot, PaneId(2));
        assert_eq!(
            transient_press(&mut keys, "S", now),
            TransientDispatch::Inactive
        );
        keys.select_pane(&snapshot, PaneId(1));
        assert_eq!(
            transient_press(&mut keys, "S", now),
            TransientDispatch::Inactive
        );
        // Successful reload clears old VM IDs, allowing a new VM to reuse them.
        keys.reset_transients();
        keys.replace_registry(&snapshot);
        assert!(matches!(
            transient_press(&mut keys, "S", now),
            TransientDispatch::Invoke { id: 10, .. }
        ));
    }

    #[test]
    fn transient_repeat_maps_keep_bound_keys_and_consume_cancellation_or_mismatch() {
        let now = Instant::now();
        for cancel in ["ESCAPE", "CTRL+G", "UNBOUND"] {
            let mut keys = KeySequences::new(&transient_snapshot(false));
            for _ in 0..2 {
                assert_eq!(
                    transient_press(&mut keys, "S", now),
                    TransientDispatch::Invoke {
                        id: 10,
                        key: Some("S".into())
                    }
                );
            }
            assert_eq!(
                transient_press(&mut keys, cancel, now),
                TransientDispatch::Invoke { id: 10, key: None }
            );
            assert_eq!(
                transient_press(&mut keys, "S", now),
                TransientDispatch::Inactive
            );
        }
    }

    #[test]
    fn transient_sequences_observe_timeout_repeat_focus_and_physical_priority() {
        let mut snapshot = transient_snapshot(false);
        snapshot.config.key_sequence_timeout_ms = Some(20);
        let mut keys = KeySequences::new(&snapshot);
        let now = Instant::now();
        assert_eq!(
            transient_press(&mut keys, "CTRL+X", now),
            TransientDispatch::Consumed
        );
        assert_eq!(
            keys.advance_transient(&["B".into()], PaneId(1), true, now),
            TransientDispatch::Consumed
        );
        assert_eq!(
            transient_press(&mut keys, "B", now),
            TransientDispatch::Invoke {
                id: 10,
                key: Some("CTRL+X B".into())
            }
        );
        assert_eq!(
            keys.advance_transient(&["PHYSICAL:KEYH".into(), "S".into()], PaneId(1), false, now),
            TransientDispatch::Invoke {
                id: 10,
                key: Some("PHYSICAL:KEYH".into())
            }
        );
        transient_press(&mut keys, "CTRL+X", now);
        keys.focus_lost();
        assert_eq!(
            transient_press(&mut keys, "S", now),
            TransientDispatch::Invoke {
                id: 10,
                key: Some("S".into())
            }
        );
        transient_press(&mut keys, "CTRL+X", now);
        assert_eq!(
            transient_press(&mut keys, "B", now + Duration::from_millis(21)),
            TransientDispatch::Invoke { id: 10, key: None }
        );
    }

    #[test]
    fn transient_stack_resumes_lower_layers_without_waiting_for_ruby() {
        let mut snapshot = transient_snapshot(false);
        let mut local = snapshot.keymap_bindings[&KeymapScope::Global].clone();
        local.transients.insert(
            0,
            TransientKeymap {
                id: 11,
                once: true,
                keys: HashSet::from(["L".into()]),
            },
        );
        snapshot
            .keymap_bindings
            .insert(KeymapScope::Pane(PaneId(1)), local);
        let mut keys = KeySequences::new(&snapshot);
        keys.select_pane(&snapshot, PaneId(1));
        let now = Instant::now();
        assert_eq!(
            transient_press(&mut keys, "L", now),
            TransientDispatch::Invoke {
                id: 11,
                key: Some("L".into())
            }
        );
        assert_eq!(
            transient_press(&mut keys, "S", now),
            TransientDispatch::Invoke {
                id: 10,
                key: Some("S".into())
            }
        );
        keys.select_pane(&snapshot, PaneId(2));
        assert_eq!(
            keys.advance_transient(&["S".into()], PaneId(2), false, now),
            TransientDispatch::Invoke {
                id: 10,
                key: Some("S".into())
            }
        );
    }

    #[test]
    fn modifier_presses_preserve_sequences_and_transients_unless_explicitly_bound() {
        let now = Instant::now();
        let mut keys = KeySequences::new(&transient_snapshot(false));
        assert_eq!(
            transient_press(&mut keys, "CTRL+PHYSICAL:CONTROLLEFT", now),
            TransientDispatch::Consumed
        );
        transient_press(&mut keys, "CTRL+X", now);
        assert_eq!(
            transient_press(&mut keys, "PHYSICAL:SHIFTRIGHT", now),
            TransientDispatch::Consumed
        );
        assert_eq!(
            transient_press(&mut keys, "B", now),
            TransientDispatch::Invoke {
                id: 10,
                key: Some("CTRL+X B".into())
            }
        );
        keys.transients[0].keys.insert("PHYSICAL:SHIFTLEFT".into());
        assert_eq!(
            transient_press(&mut keys, "PHYSICAL:SHIFTLEFT", now),
            TransientDispatch::Invoke {
                id: 10,
                key: Some("PHYSICAL:SHIFTLEFT".into())
            }
        );

        let mut keys = KeySequences::new(&snapshot());
        press(&mut keys, "CTRL+X", now);
        press(&mut keys, "CTRL+PHYSICAL:CONTROLRIGHT", now);
        assert_eq!(
            press(&mut keys, "CTRL+F", now),
            SequenceDispatch::Binding("CTRL+X CTRL+F".into())
        );
    }

    #[test]
    fn resolves_shared_prefixes_and_multiple_steps_without_ruby() {
        let mut keys = KeySequences::new(&snapshot());
        let now = Instant::now();
        assert_eq!(press(&mut keys, "A", now), SequenceDispatch::Unassigned);
        assert_eq!(press(&mut keys, "CTRL+X", now), SequenceDispatch::Consumed);
        assert_eq!(
            press(&mut keys, "CTRL+F", now),
            SequenceDispatch::Binding("CTRL+X CTRL+F".into())
        );
        assert_eq!(press(&mut keys, "CTRL+X", now), SequenceDispatch::Consumed);
        assert_eq!(press(&mut keys, "R", now), SequenceDispatch::Consumed);
        assert_eq!(
            press(&mut keys, "B", now),
            SequenceDispatch::Binding("CTRL+X R B".into())
        );
    }

    #[test]
    fn repeats_do_not_advance_or_extend_the_deadline() {
        let mut keys = KeySequences::new(&snapshot());
        let now = Instant::now();
        press(&mut keys, "CTRL+X", now);
        assert_eq!(
            keys.advance(
                &["CTRL+X".into()],
                PaneId(1),
                true,
                false,
                now + Duration::from_millis(900)
            ),
            SequenceDispatch::Consumed
        );
        assert_eq!(
            press(&mut keys, "CTRL+F", now + Duration::from_millis(1001)),
            SequenceDispatch::Unassigned
        );
        assert_eq!(
            keys.advance(&["CTRL+X".into()], PaneId(1), true, false, now),
            SequenceDispatch::Consumed
        );
        assert_eq!(
            press(&mut keys, "CTRL+F", now),
            SequenceDispatch::Unassigned
        );
    }

    #[test]
    fn cancellation_and_invalid_suffixes_do_not_reach_the_pty() {
        let mut keys = KeySequences::new(&snapshot());
        let now = Instant::now();
        for suffix in ["ESCAPE", "CTRL+G", "UNKNOWN"] {
            press(&mut keys, "CTRL+X", now);
            assert_eq!(press(&mut keys, suffix, now), SequenceDispatch::Consumed);
            assert_eq!(
                press(&mut keys, "CTRL+F", now),
                SequenceDispatch::Unassigned
            );
        }
    }

    #[test]
    fn scope_changes_and_explicit_clearing_cancel_pending_prefixes() {
        let mut keys = KeySequences::new(&snapshot());
        let now = Instant::now();
        press(&mut keys, "CTRL+X", now);
        assert_eq!(
            keys.advance(&["CTRL+F".into()], PaneId(2), false, false, now),
            SequenceDispatch::Unassigned
        );
        press(&mut keys, "CTRL+X", now);
        keys.clear();
        assert_eq!(
            press(&mut keys, "CTRL+F", now),
            SequenceDispatch::Unassigned
        );
    }

    #[test]
    fn physical_candidates_win_and_timeout_can_be_disabled() {
        let mut snapshot = snapshot();
        snapshot.config.key_sequence_timeout_ms = None;
        let mut keys = KeySequences::new(&snapshot);
        let now = Instant::now();
        press(&mut keys, "CTRL+X", now);
        assert_eq!(
            keys.advance(
                &["PHYSICAL:KEYH".into(), "CTRL+F".into()],
                PaneId(1),
                false,
                false,
                now + Duration::from_secs(3600)
            ),
            SequenceDispatch::Binding("CTRL+X PHYSICAL:KEYH".into())
        );
    }

    #[test]
    fn single_physical_bindings_keep_priority_over_logical_prefixes() {
        let mut snapshot = snapshot();
        snapshot
            .native_actions
            .insert("PHYSICAL:KEYX".into(), NativeAction::NewTab);
        let mut keys = KeySequences::new(&snapshot);
        let candidates = ["PHYSICAL:KEYX".into(), "CTRL+X".into()];
        assert_eq!(
            keys.advance(&candidates, PaneId(1), false, false, Instant::now()),
            SequenceDispatch::Unassigned
        );
        snapshot
            .native_actions
            .insert("PHYSICAL:KEYX".into(), NativeAction::YankSelection);
        keys.replace_registry(&snapshot);
        assert_eq!(
            keys.advance(&candidates, PaneId(1), false, false, Instant::now()),
            SequenceDispatch::Consumed
        );
        keys.clear();
        assert_eq!(
            keys.advance(&candidates, PaneId(1), false, true, Instant::now()),
            SequenceDispatch::Unassigned
        );
    }

    #[test]
    fn registry_changes_cancel_prefixes_and_preserve_repeat_suppression_until_release() {
        use winit::keyboard::KeyCode;
        let snapshot = snapshot();
        let mut keys = KeySequences::new(&snapshot);
        let physical = PhysicalKey::Code(KeyCode::KeyF);
        keys.hold(physical);
        press(&mut keys, "CTRL+X", Instant::now());
        keys.replace_registry(&snapshot);
        assert!(keys.is_held(physical));
        assert_eq!(
            press(&mut keys, "CTRL+F", Instant::now()),
            SequenceDispatch::Unassigned
        );
        keys.release(physical);
        assert!(!keys.is_held(physical));
        keys.hold(physical);
        keys.focus_lost();
        assert!(!keys.is_held(physical));
    }

    #[test]
    fn pane_layers_override_lower_physical_bindings_and_recompile_on_scope_changes() {
        use toyoterm_script::KeymapBindings;
        let mut snapshot = snapshot();
        snapshot.keymap_bindings.insert(
            KeymapScope::Pane(PaneId(4)),
            KeymapBindings {
                native_actions: HashMap::from([
                    ("PHYSICAL:KEYX".into(), NativeAction::CloseTab),
                    ("CTRL+X F".into(), NativeAction::NewTab),
                ]),
                priorities: HashMap::from([("PHYSICAL:KEYX".into(), 1), ("CTRL+X F".into(), 2)]),
                ..KeymapBindings::default()
            },
        );
        let mut keys = KeySequences::new(&snapshot);
        keys.select_pane(&snapshot, PaneId(4));
        let now = Instant::now();
        assert_eq!(
            keys.advance(
                &["PHYSICAL:KEYX".into(), "CTRL+X".into()],
                PaneId(4),
                false,
                false,
                now
            ),
            SequenceDispatch::Consumed
        );
        assert_eq!(
            keys.advance(&["F".into()], PaneId(4), false, false, now),
            SequenceDispatch::Binding("CTRL+X F".into())
        );
        keys.select_pane(&snapshot, PaneId(5));
        assert_eq!(
            keys.advance(&["CTRL+X".into()], PaneId(5), false, false, now),
            SequenceDispatch::Consumed
        );
        keys.select_pane(&snapshot, PaneId(4));
        assert_eq!(
            keys.advance(&["CTRL+F".into()], PaneId(4), false, false, now),
            SequenceDispatch::Unassigned
        );
    }
}
