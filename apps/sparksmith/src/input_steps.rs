//! What a person did, in the order they did it, where it matters: what is
//! typed after a click comes a frame after the click.
//!
//! While a frame runs long (a board is solved in it), the next click and
//! what is typed after it reach the app together, in one frame. A field
//! opened by that click has no keyboard until the frame after, so its first
//! characters would go nowhere, or to a list's type-ahead. [`Steps`] ends
//! the frame there, and after each key that can move the keyboard, and keeps
//! the rest for the next frame. Pointer input alone (a double-click, a drag,
//! a click with a modifier) and a held key's repeats come as they always did.
use egui::{Event, RawInput};

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
        let end = step(&self.waiting, typing);
        let rest = self.waiting.split_off(end);
        raw.events = std::mem::replace(&mut self.waiting, rest);
    }

    /// Whether the next frame has a step already: it is due at once.
    pub(crate) fn more(&self) -> bool {
        !self.waiting.is_empty()
    }
}

/// How many of `events` are the first step. Modifier changes are events of
/// the same stream, so a step keeps the ones before it.
fn step(events: &[Event], typing: bool) -> usize {
    let mut clicked = false;
    for (i, event) in events.iter().enumerate() {
        match event {
            Event::PointerButton { .. } => clicked = true,
            Event::Key { pressed: true, .. } | Event::Text(_) | Event::Paste(_) | Event::Ime(_)
                if clicked =>
            {
                return i;
            }
            // A held key's repeats would pile up faster than slow frames
            // take them.
            Event::Key {
                pressed: true,
                repeat: false,
                ..
            } => {
                let text = events
                    .iter()
                    .skip(i + 1)
                    .take_while(|e| matches!(e, Event::Text(_)))
                    .count();
                // A character typed in a field leaves the keyboard there.
                if !(typing && text > 0) {
                    return i + 1 + text;
                }
            }
            _ => {}
        }
    }
    events.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Key, Modifiers, PointerButton, pos2};

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

    fn lengths(events: Vec<Event>, typing: bool) -> Vec<usize> {
        let out = frames(events.clone(), typing);
        assert_eq!(out.concat(), events, "nothing is lost or reordered");
        out.iter().map(Vec::len).collect()
    }

    #[test]
    fn what_is_typed_after_a_click_waits_a_frame() {
        let events = vec![
            button(true),
            button(false),
            press(Key::Num6),
            Event::Text("6".into()),
            press(Key::Enter),
        ];
        assert_eq!(lengths(events, false), [2, 2, 1]);
    }

    #[test]
    fn characters_typed_in_a_field_share_a_frame_up_to_enter() {
        let events = vec![
            press(Key::Num6),
            Event::Text("6".into()),
            press(Key::Num7),
            Event::Text("7".into()),
            press(Key::Enter),
            button(true),
            button(false),
        ];
        assert_eq!(lengths(events, true), [5, 2]);
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
        assert_eq!(lengths(vec![repeat; 20], false), [20]);
    }

    /// Double-clicks, drags and clicks with a modifier are one step, as they
    /// arrived.
    #[test]
    fn pointer_input_alone_is_never_split() {
        let moved = |x| Event::PointerMoved(pos2(x, 1.0));
        let double = vec![button(true), button(false), button(true), button(false)];
        assert_eq!(lengths(double, false), [4]);
        let drag = vec![button(true), moved(5.0), moved(40.0), button(false)];
        assert_eq!(lengths(drag, false), [4]);
        let command = vec![
            Event::ModifiersChanged(Modifiers::COMMAND),
            moved(1.0),
            button(true),
            button(false),
            Event::ModifiersChanged(Modifiers::NONE),
        ];
        assert_eq!(lengths(command, false), [5]);
    }
}
