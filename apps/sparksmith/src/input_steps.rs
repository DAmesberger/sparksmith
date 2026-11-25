//! What a person did, in the order they did it: one step per frame.
//!
//! While a frame runs long (a board is solved in it), the next click and
//! what is typed after it reach the app together, in one frame. A field
//! opened by that click has no keyboard until the frame after, so its first
//! characters would go nowhere, or to a list's type-ahead. [`Steps`] ends
//! the frame after each click and each key that moves the keyboard, and
//! keeps the rest for the next frame.
use egui::{Event, Modifiers, RawInput};

/// Input events waiting for their frame.
#[derive(Debug, Default)]
pub(crate) struct Steps {
    waiting: Vec<Event>,
}

impl Steps {
    /// Give `raw` the next step of what was waiting and what it brings;
    /// `typing` when a text field has the keyboard.
    pub(crate) fn next(&mut self, raw: &mut RawInput, typing: bool) {
        self.waiting.append(&mut raw.events);
        let (end, modifiers) = step(&self.waiting, typing);
        let rest = self.waiting.split_off(end);
        raw.events = std::mem::replace(&mut self.waiting, rest);
        // The modifiers are as they were at the step, not after the rest.
        if let Some(modifiers) = modifiers.filter(|_| self.more()) {
            raw.modifiers = modifiers;
        }
    }

    /// Whether the next frame has a step already: it is due at once.
    pub(crate) fn more(&self) -> bool {
        !self.waiting.is_empty()
    }
}

/// How many of `events` are the first step, and the modifiers held at its
/// end. A held key's repeats are no steps of their own: they would pile up
/// faster than slow frames take them.
fn step(events: &[Event], typing: bool) -> (usize, Option<Modifiers>) {
    for (i, event) in events.iter().enumerate() {
        match event {
            Event::PointerButton { modifiers, .. } => return (i + 1, Some(*modifiers)),
            Event::Key {
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } => {
                let text = events
                    .iter()
                    .skip(i + 1)
                    .take_while(|e| matches!(e, Event::Text(_)))
                    .count();
                // A character typed in a field leaves the keyboard there.
                if !(typing && text > 0) {
                    return (i + 1 + text, Some(*modifiers));
                }
            }
            _ => {}
        }
    }
    (events.len(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Key, PointerButton, pos2};

    fn press(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn button(pressed: bool) -> Event {
        Event::PointerButton {
            pos: pos2(1.0, 1.0),
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::SHIFT,
        }
    }

    /// The frames `events` are given in.
    fn frames(events: Vec<Event>, typing: bool) -> Vec<Vec<Event>> {
        let mut steps = Steps::default();
        let mut raw = RawInput {
            events,
            ..Default::default()
        };
        let mut out = Vec::new();
        loop {
            steps.next(&mut raw, typing);
            out.push(std::mem::take(&mut raw.events));
            if !steps.more() {
                return out;
            }
        }
    }

    #[test]
    fn a_click_ends_its_frame_and_nothing_is_lost_or_reordered() {
        let events = vec![
            button(true),
            button(false),
            press(Key::Num6),
            Event::Text("6".into()),
            press(Key::Enter),
        ];
        let out = frames(events.clone(), false);
        assert_eq!(out.iter().map(Vec::len).collect::<Vec<_>>(), [1, 1, 2, 1]);
        assert_eq!(out.concat(), events);
    }

    #[test]
    fn characters_typed_in_a_field_share_a_frame_up_to_enter() {
        let events = vec![
            press(Key::Num6),
            Event::Text("6".into()),
            press(Key::Num7),
            Event::Text("7".into()),
            press(Key::Enter),
            press(Key::Tab),
        ];
        let out = frames(events, true);
        assert_eq!(out.iter().map(Vec::len).collect::<Vec<_>>(), [5, 1]);
    }

    #[test]
    fn a_held_key_does_not_pile_up() {
        let repeat = Event::Key {
            key: Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(frames(vec![repeat; 20], false).len(), 1);
    }

    #[test]
    fn a_step_keeps_the_modifiers_it_had() {
        let mut steps = Steps::default();
        let mut raw = RawInput {
            events: vec![button(true), button(false)],
            ..Default::default()
        };
        steps.next(&mut raw, false);
        assert_eq!(raw.modifiers, Modifiers::SHIFT);
    }
}
