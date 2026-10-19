//! Terminal keys to Doom keys, and the key-down/key-up stream Doom needs.
//!
//! Doom reads a held key as a key-down and, later, a key-up. A terminal
//! reports presses (and, while a key is held, the keyboard's auto-repeat as
//! more presses); only a terminal with the kitty keyboard protocol also
//! reports releases. So there are two ways a key comes up:
//!
//! - **Real releases.** When a release arrives for a key, the key is known
//!   to report them, and from then on it is held until its release comes.
//! - **Synthesized releases.** For a key never seen released (every key in
//!   a legacy terminal, and Enter, Tab and Backspace even in kitty), the key
//!   is let go after a hold time: long after the first press, to bridge the
//!   keyboard's auto-repeat delay, and short after each repeat. Menu keys
//!   are taps: down for a few tics, then up.
//!
//! Modifier keys pressed alone are never reported to a terminal program,
//! so Ctrl (fire), Shift (run) and Alt (strafe) are read from the modifiers
//! of the other keys' events and held while those keep coming.

use std::collections::{HashMap, HashSet, VecDeque};

use rich_interact::{Key, KeyCode};

/// Doom's key codes (doomkeys.h).
pub mod doom {
    pub const RIGHT: u8 = 0xae;
    pub const LEFT: u8 = 0xac;
    pub const UP: u8 = 0xad;
    pub const DOWN: u8 = 0xaf;
    pub const STRAFE_L: u8 = 0xa0;
    pub const STRAFE_R: u8 = 0xa1;
    pub const USE: u8 = 0xa2;
    pub const FIRE: u8 = 0xa3;
    pub const ESCAPE: u8 = 27;
    pub const ENTER: u8 = 13;
    pub const TAB: u8 = 9;
    pub const BACKSPACE: u8 = 0x7f;
    pub const RSHIFT: u8 = 0x80 + 0x36;
    pub const RALT: u8 = 0x80 + 0x38;
    pub const PAUSE: u8 = 0xff;

    /// F1 to F12.
    pub fn f(n: u8) -> Option<u8> {
        match n {
            1..=10 => Some(0x80 + 0x3a + n),
            11 => Some(0x80 + 0x57),
            12 => Some(0x80 + 0x58),
            _ => None,
        }
    }
}

/// How a key is held when its release is not reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hold {
    /// Menu keys, weapons, use: down for a few tics, then up.
    Tap,
    /// Movement and fire: held across the keyboard's auto-repeat.
    Move,
    /// Derived from a modifier on another key's event.
    Modifier,
}

/// What the terminal side sends the engine thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// A key went down (or auto-repeated).
    Down { key: u8, hold: Hold },
    /// A key came up. `real` when the terminal reported the release (it
    /// teaches that the key reports them); not for a modifier let go.
    Up { key: u8, real: bool },
}

/// Hold times for keys whose releases are not reported, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoldTimes {
    /// After a first press: longer than the keyboard's auto-repeat delay.
    pub first: u64,
    /// After an auto-repeated press: longer than the repeat interval.
    pub repeat: u64,
    /// A tap: long enough for the engine to see it in a tic (28.6 ms).
    pub tap: u64,
}

impl Default for HoldTimes {
    fn default() -> Self {
        HoldTimes {
            first: 400,
            repeat: 120,
            tap: 80,
        }
    }
}

/// The Doom keys for a terminal key, and how each is held.
pub fn map_key(key: &Key) -> Option<(u8, Hold)> {
    use doom::*;
    Some(match key.code {
        KeyCode::Up => (UP, Hold::Move),
        KeyCode::Down => (DOWN, Hold::Move),
        KeyCode::Left => (LEFT, Hold::Move),
        KeyCode::Right => (RIGHT, Hold::Move),
        KeyCode::Enter => (ENTER, Hold::Tap),
        KeyCode::Escape => (ESCAPE, Hold::Tap),
        KeyCode::Tab => (TAB, Hold::Tap),
        KeyCode::Backspace => (BACKSPACE, Hold::Tap),
        KeyCode::F(n) => (f(n)?, Hold::Tap),
        KeyCode::Char(c) => match c {
            ' ' => (USE, Hold::Tap),
            ',' | '<' => (STRAFE_L, Hold::Move),
            '.' | '>' => (STRAFE_R, Hold::Move),
            // Fire without Ctrl: keys no cheat code uses.
            'x' | 'X' | 'z' | 'Z' => (FIRE, Hold::Move),
            c if c.is_ascii_alphanumeric() => (c.to_ascii_lowercase() as u8, Hold::Tap),
            '-' | '=' | '[' | ']' | '/' | '\'' | ';' | '`' | '\\' => (c as u8, Hold::Tap),
            '+' => (b'=', Hold::Tap),
            '_' => (b'-', Hold::Tap),
            _ => return None,
        },
        _ => return None,
    })
}

/// The Doom keys the modifiers stand for: Ctrl fires, Shift runs, Alt
/// strafes.
pub const MODIFIER_KEYS: [u8; 3] = [doom::FIRE, doom::RSHIFT, doom::RALT];

/// The terminal side: turns key events into [`Input`]s, tracking which
/// modifier-derived keys it holds.
#[derive(Debug, Default)]
pub struct Translator {
    modifiers: [bool; 3],
}

impl Translator {
    pub fn new() -> Translator {
        Translator::default()
    }

    /// A key pressed (or repeated): what to send.
    pub fn press(&mut self, key: &Key) -> Vec<Input> {
        let mut out = Vec::new();
        let held = [key.modifiers.ctrl, key.modifiers.shift, key.modifiers.alt];
        // An upper-case letter says Shift, too.
        let shifted = matches!(key.code, KeyCode::Char(c) if c.is_ascii_uppercase());
        for (index, &doom_key) in MODIFIER_KEYS.iter().enumerate() {
            let now = held[index] || (index == 1 && shifted);
            if now {
                out.push(Input::Down {
                    key: doom_key,
                    hold: Hold::Modifier,
                });
            } else if self.modifiers[index] {
                out.push(Input::Up {
                    key: doom_key,
                    real: false,
                });
            }
            self.modifiers[index] = now;
        }
        if let Some((doom_key, hold)) = map_key(key) {
            out.push(Input::Down {
                key: doom_key,
                hold,
            });
        }
        out
    }

    /// A key released (kitty keyboard protocol only).
    pub fn release(&mut self, key: &Key) -> Vec<Input> {
        map_key(key)
            .map(|(doom_key, _)| Input::Up {
                key: doom_key,
                real: true,
            })
            .into_iter()
            .collect()
    }
}

/// The engine side: the keys Doom holds, and the events to give it.
#[derive(Debug, Default)]
pub struct KeyState {
    times: HoldTimes,
    /// Held keys, with when to let go (`None`: when the release comes).
    held: HashMap<u8, Option<u64>>,
    /// Keys seen released by the terminal.
    reports_release: HashSet<u8>,
    /// Events for Doom, oldest first: `(pressed, key)`.
    queue: VecDeque<(bool, u8)>,
}

impl KeyState {
    pub fn new(times: HoldTimes) -> KeyState {
        KeyState {
            times,
            ..KeyState::default()
        }
    }

    /// Take an input at `now` (milliseconds on the engine's clock).
    pub fn input(&mut self, input: Input, now: u64) {
        match input {
            Input::Down { key, hold } => {
                let repeat = self.held.contains_key(&key);
                let deadline = if hold != Hold::Modifier && self.reports_release.contains(&key) {
                    None
                } else {
                    let wait = match hold {
                        Hold::Tap => self.times.tap,
                        Hold::Move | Hold::Modifier if repeat => self.times.repeat,
                        Hold::Move | Hold::Modifier => self.times.first,
                    };
                    Some(now + wait)
                };
                if !repeat {
                    self.queue.push_back((true, key));
                }
                // A tap's repeat keeps its first deadline: the menu sees
                // repeats as more taps, after the tap is over.
                match self.held.get_mut(&key) {
                    Some(current) if hold == Hold::Tap => {
                        if current.is_none() {
                            *current = deadline;
                        }
                    }
                    Some(current) => *current = deadline,
                    None => {
                        self.held.insert(key, deadline);
                    }
                }
            }
            Input::Up { key, real } => {
                if real {
                    self.reports_release.insert(key);
                }
                if self.held.remove(&key).is_some() {
                    self.queue.push_back((false, key));
                }
            }
        }
    }

    /// The next event for Doom at `now`, letting go of keys whose hold ran
    /// out first.
    pub fn next_event(&mut self, now: u64) -> Option<(bool, u8)> {
        if self.queue.is_empty() {
            let mut expired: Vec<u8> = self
                .held
                .iter()
                .filter(|(_, deadline)| deadline.is_some_and(|d| d <= now))
                .map(|(key, _)| *key)
                .collect();
            expired.sort_unstable();
            for key in expired {
                self.held.remove(&key);
                self.queue.push_back((false, key));
            }
        }
        self.queue.pop_front()
    }

    /// Whether Doom holds `key` now.
    pub fn is_held(&self, key: u8) -> bool {
        self.held.contains_key(&key)
    }

    /// Whether the terminal has shown it reports `key`'s releases.
    pub fn knows_release(&self, key: u8) -> bool {
        self.reports_release.contains(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rich_interact::Modifiers;

    fn drain(state: &mut KeyState, now: u64) -> Vec<(bool, u8)> {
        std::iter::from_fn(|| state.next_event(now)).collect()
    }

    #[test]
    fn maps_the_documented_keys() {
        let k = |code| map_key(&Key::new(code)).map(|(key, _)| key);
        assert_eq!(k(KeyCode::Up), Some(doom::UP));
        assert_eq!(k(KeyCode::Left), Some(doom::LEFT));
        assert_eq!(k(KeyCode::Enter), Some(doom::ENTER));
        assert_eq!(k(KeyCode::Escape), Some(doom::ESCAPE));
        assert_eq!(k(KeyCode::Tab), Some(doom::TAB));
        assert_eq!(k(KeyCode::F(1)), Some(0xbb));
        assert_eq!(k(KeyCode::F(10)), Some(0xc4));
        assert_eq!(k(KeyCode::F(11)), Some(0xd7));
        assert_eq!(k(KeyCode::F(13)), None);
        assert_eq!(k(KeyCode::Char(' ')), Some(doom::USE));
        assert_eq!(k(KeyCode::Char('Y')), Some(b'y'));
        assert_eq!(k(KeyCode::Char('n')), Some(b'n'));
        assert_eq!(k(KeyCode::Char('3')), Some(b'3'));
        assert_eq!(k(KeyCode::Char(',')), Some(doom::STRAFE_L));
        assert_eq!(k(KeyCode::Char('.')), Some(doom::STRAFE_R));
        assert_eq!(k(KeyCode::Char('x')), Some(doom::FIRE));
        assert_eq!(k(KeyCode::Char('é')), None);
        assert_eq!(k(KeyCode::PageUp), None);
    }

    #[test]
    fn modifiers_become_fire_run_and_strafe() {
        let mut t = Translator::new();
        let ctrl_up = Key::with(KeyCode::Up, Modifiers::CTRL);
        assert_eq!(
            t.press(&ctrl_up),
            vec![
                Input::Down {
                    key: doom::FIRE,
                    hold: Hold::Modifier
                },
                Input::Down {
                    key: doom::UP,
                    hold: Hold::Move
                },
            ]
        );
        // The next event without Ctrl lets fire go.
        assert_eq!(
            t.press(&Key::new(KeyCode::Up)),
            vec![
                Input::Up {
                    key: doom::FIRE,
                    real: false
                },
                Input::Down {
                    key: doom::UP,
                    hold: Hold::Move
                },
            ]
        );
        let shift = Modifiers {
            shift: true,
            ..Modifiers::NONE
        };
        let ins = t.press(&Key::with(KeyCode::Left, shift));
        assert!(ins.contains(&Input::Down {
            key: doom::RSHIFT,
            hold: Hold::Modifier
        }));
        assert_eq!(
            t.release(&Key::new(KeyCode::Left)),
            vec![Input::Up {
                key: doom::LEFT,
                real: true
            }]
        );
    }

    #[test]
    fn synthesizes_releases_across_auto_repeat() {
        let mut s = KeyState::new(HoldTimes::default());
        let up = Input::Down {
            key: doom::UP,
            hold: Hold::Move,
        };
        s.input(up, 0);
        assert_eq!(drain(&mut s, 0), vec![(true, doom::UP)]);
        // Still held through the auto-repeat delay.
        assert_eq!(drain(&mut s, 399), vec![]);
        // Repeats keep it down with the short hold, sending nothing new.
        s.input(up, 390);
        s.input(up, 420);
        assert_eq!(drain(&mut s, 500), vec![]);
        assert!(s.is_held(doom::UP));
        // The repeats stop: up 120 ms after the last.
        assert_eq!(drain(&mut s, 540), vec![(false, doom::UP)]);
        assert!(!s.is_held(doom::UP));
    }

    #[test]
    fn taps_are_short_and_repeat_as_taps() {
        let mut s = KeyState::new(HoldTimes::default());
        let enter = Input::Down {
            key: doom::ENTER,
            hold: Hold::Tap,
        };
        s.input(enter, 0);
        s.input(enter, 30);
        assert_eq!(drain(&mut s, 30), vec![(true, doom::ENTER)]);
        assert_eq!(drain(&mut s, 80), vec![(false, doom::ENTER)]);
        s.input(enter, 100);
        assert_eq!(drain(&mut s, 100), vec![(true, doom::ENTER)]);
    }

    #[test]
    fn a_reported_release_is_trusted_from_then_on() {
        let mut s = KeyState::new(HoldTimes::default());
        let left = Input::Down {
            key: doom::LEFT,
            hold: Hold::Move,
        };
        s.input(left, 0);
        s.input(
            Input::Up {
                key: doom::LEFT,
                real: true,
            },
            100,
        );
        assert_eq!(drain(&mut s, 100), vec![(true, doom::LEFT), (false, doom::LEFT)]);
        assert!(s.knows_release(doom::LEFT));
        // Now held as long as it is, with no timeout.
        s.input(left, 1000);
        assert_eq!(drain(&mut s, 60_000), vec![(true, doom::LEFT)]);
        s.input(
            Input::Up {
                key: doom::LEFT,
                real: true,
            },
            60_001,
        );
        assert_eq!(drain(&mut s, 60_001), vec![(false, doom::LEFT)]);
        // A release for a key not held sends nothing.
        s.input(
            Input::Up {
                key: doom::LEFT,
                real: true,
            },
            60_002,
        );
        assert_eq!(drain(&mut s, 60_002), vec![]);
    }

    #[test]
    fn modifiers_always_time_out() {
        let mut s = KeyState::new(HoldTimes::default());
        s.input(
            Input::Up {
                key: doom::FIRE,
                real: true,
            },
            0,
        );
        s.input(
            Input::Down {
                key: doom::FIRE,
                hold: Hold::Modifier,
            },
            0,
        );
        assert_eq!(drain(&mut s, 0), vec![(true, doom::FIRE)]);
        assert_eq!(drain(&mut s, 400), vec![(false, doom::FIRE)]);
    }
}
