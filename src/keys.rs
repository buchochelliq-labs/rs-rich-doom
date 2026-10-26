//! Terminal keys to Doom keys, and the key-down/key-up stream Doom needs.
//!
//! Doom reads a held key as a key-down and, later, a key-up. Which keys
//! are held is the app's to say: intuiTUIve's held keys
//! ([`Ctx::held_keys`](intuituive::Ctx::held_keys)) follow the releases a
//! terminal with the kitty keyboard protocol reports, and make up the
//! others' after a hold time ([`App::synthesize_releases`]). On each tic
//! ([`Keyboard::step`]) the Doom keys those stand for are compared with
//! the ones Doom holds, and the difference is sent as downs and ups.
//!
//! Keys of two kinds:
//!
//! - **Held** keys (movement, fire, strafe, run) are down while a key that
//!   stands for them is held. Several terminal keys can stand for one Doom
//!   key (`x`, `z` and Ctrl all fire): it is down while any of them is.
//! - **Taps** (menus, weapons, use, letters for cheats and save names) are
//!   a down on the tic after the press and an up on the tic after that,
//!   once per press: auto-repeat makes more taps, so menus scroll and a
//!   cheat's double letter counts twice.
//!
//! With the kitty protocol's every-key mode
//! ([`App::all_keys`](intuituive::App::all_keys)) Ctrl, Shift and Alt are
//! reported alone and held as Doom's fire, run and strafe. In other
//! terminals they come only as the modifiers of another key's event, so
//! they are held while a held key last pressed with them is.
//!
//! [`App::synthesize_releases`]: intuituive::App::synthesize_releases

use intuituive::plain_key;
use rich_interact::{Key, KeyCode, ModifierKey, Modifiers};

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

/// How a key reaches Doom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hold {
    /// Menu keys, weapons, use, letters: one down and up a press.
    Tap,
    /// Movement and fire: down while held.
    Move,
}

/// An event for Doom, in the order it is to see them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Down(u8),
    Up(u8),
}

/// The Doom key for a terminal key, and how it reaches Doom.
pub fn map_key(key: &Key) -> Option<(u8, Hold)> {
    use doom::*;
    Some(match key.code {
        KeyCode::Up => (UP, Hold::Move),
        KeyCode::Down => (DOWN, Hold::Move),
        KeyCode::Left => (LEFT, Hold::Move),
        KeyCode::Right => (RIGHT, Hold::Move),
        KeyCode::Enter => (ENTER, Hold::Tap),
        KeyCode::Escape => (ESCAPE, Hold::Tap),
        KeyCode::Tab | KeyCode::BackTab => (TAB, Hold::Tap),
        KeyCode::Backspace => (BACKSPACE, Hold::Tap),
        KeyCode::F(n) => (f(n)?, Hold::Tap),
        KeyCode::Modifier(ModifierKey::Ctrl) => (FIRE, Hold::Move),
        KeyCode::Modifier(ModifierKey::Shift) => (RSHIFT, Hold::Move),
        KeyCode::Modifier(ModifierKey::Alt) => (RALT, Hold::Move),
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

/// The Doom keys the modifiers of a held key's press stand for: Ctrl
/// fires, Shift runs, Alt strafes.
fn modifier_keys(modifiers: Modifiers) -> impl Iterator<Item = u8> {
    [
        (modifiers.ctrl, doom::FIRE),
        (modifiers.shift, doom::RSHIFT),
        (modifiers.alt, doom::RALT),
    ]
    .into_iter()
    .filter_map(|(on, key)| on.then_some(key))
}

/// The keys Doom holds, and the taps on their way to it. Lives on the
/// app's thread: [`press`](Self::press) for each key event,
/// [`step`](Self::step) once a tic.
#[derive(Debug, Default)]
pub struct Keyboard {
    /// The Doom keys down now, from held keys, in key order.
    held: Vec<u8>,
    /// Taps pressed since the last step.
    taps: Vec<u8>,
    /// Taps sent down on the last step, to let go on this one.
    releasing: Vec<u8>,
    /// The modifiers each held key (a plain key) was last pressed with.
    modifiers: Vec<(Key, Modifiers)>,
}

impl Keyboard {
    pub fn new() -> Keyboard {
        Keyboard::default()
    }

    /// A key pressed or auto-repeated. Whether it means anything to Doom.
    pub fn press(&mut self, key: &Key) -> bool {
        let Some((doom_key, hold)) = map_key(key) else {
            return false;
        };
        match hold {
            Hold::Tap => self.taps.push(doom_key),
            Hold::Move => {
                let plain = plain_key(*key);
                let mut modifiers = key.modifiers;
                // An upper-case letter says Shift, too.
                modifiers.shift |= matches!(key.code, KeyCode::Char(c) if c.is_ascii_uppercase());
                match self.modifiers.iter_mut().find(|(k, _)| *k == plain) {
                    Some(entry) => entry.1 = modifiers,
                    None => self.modifiers.push((plain, modifiers)),
                }
            }
        }
        true
    }

    /// What Doom is to see this tic, given the keys the app holds now
    /// (plain keys, as [`held_keys`](intuituive::held_keys) has them):
    /// last tic's taps let go, this tic's taps pressed, and the held keys
    /// that changed.
    pub fn step(&mut self, held: &[Key]) -> Vec<Input> {
        // A key no longer held forgets its modifiers.
        self.modifiers
            .retain(|(key, _)| held.iter().any(|h| plain_key(*h) == *key));
        let mut want: Vec<u8> = Vec::new();
        for key in held {
            if let Some((doom_key, Hold::Move)) = map_key(key) {
                want.push(doom_key);
                let plain = plain_key(*key);
                if let Some((_, modifiers)) = self.modifiers.iter().find(|(k, _)| *k == plain) {
                    want.extend(modifier_keys(*modifiers));
                }
            }
        }
        want.sort_unstable();
        want.dedup();
        let mut out: Vec<Input> = self.releasing.drain(..).map(Input::Up).collect();
        for &key in &self.taps {
            out.push(Input::Down(key));
        }
        self.releasing = std::mem::take(&mut self.taps);
        for &key in &self.held {
            if want.binary_search(&key).is_err() {
                out.push(Input::Up(key));
            }
        }
        for &key in &want {
            if self.held.binary_search(&key).is_err() {
                out.push(Input::Down(key));
            }
        }
        self.held = want;
        out
    }

    /// Whether Doom holds `key` (a held key) now.
    pub fn is_held(&self, key: u8) -> bool {
        self.held.binary_search(&key).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> Key {
        Key::parse(name).unwrap()
    }

    fn plain(names: &[&str]) -> Vec<Key> {
        names.iter().map(|n| plain_key(key(n))).collect()
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
        assert_eq!(k(KeyCode::Modifier(ModifierKey::Ctrl)), Some(doom::FIRE));
        assert_eq!(k(KeyCode::Modifier(ModifierKey::Shift)), Some(doom::RSHIFT));
        assert_eq!(k(KeyCode::Modifier(ModifierKey::Super)), None);
        assert_eq!(k(KeyCode::Char('é')), None);
        assert_eq!(k(KeyCode::PageUp), None);
    }

    #[test]
    fn held_keys_go_down_and_up_with_the_app() {
        let mut k = Keyboard::new();
        assert!(k.press(&key("up")));
        assert_eq!(k.step(&plain(&["up"])), vec![Input::Down(doom::UP)]);
        // Held: nothing new, repeats or not.
        k.press(&key("up"));
        assert_eq!(k.step(&plain(&["up"])), vec![]);
        assert!(k.is_held(doom::UP));
        assert_eq!(k.step(&[]), vec![Input::Up(doom::UP)]);
        assert!(!k.is_held(doom::UP));
    }

    #[test]
    fn two_keys_for_one_action_hold_it_until_both_are_up() {
        // The review's case: x held, z pressed, x let go: still firing.
        let mut k = Keyboard::new();
        k.press(&key("x"));
        assert_eq!(k.step(&plain(&["x"])), vec![Input::Down(doom::FIRE)]);
        k.press(&key("z"));
        assert_eq!(k.step(&plain(&["x", "z"])), vec![]);
        assert_eq!(k.step(&plain(&["z"])), vec![], "z still fires");
        assert_eq!(k.step(&[]), vec![Input::Up(doom::FIRE)]);
    }

    #[test]
    fn modifiers_come_from_the_press_or_on_their_own() {
        let mut k = Keyboard::new();
        // A legacy terminal: Ctrl only as a modifier of the arrow.
        k.press(&key("ctrl+up"));
        assert_eq!(
            k.step(&plain(&["up"])),
            vec![Input::Down(doom::FIRE), Input::Down(doom::UP)]
        );
        // The arrow repeats without Ctrl: fire is let go, the arrow kept.
        k.press(&key("up"));
        assert_eq!(k.step(&plain(&["up"])), vec![Input::Up(doom::FIRE)]);
        k.step(&[]);
        // The kitty protocol's every-key mode: Ctrl alone.
        let ctrl = Key::new(KeyCode::Modifier(ModifierKey::Ctrl));
        k.press(&ctrl);
        assert_eq!(k.step(&[ctrl]), vec![Input::Down(doom::FIRE)]);
        assert_eq!(k.step(&[]), vec![Input::Up(doom::FIRE)]);
        // Shift with an arrow runs.
        k.press(&key("shift+left"));
        assert!(k
            .step(&plain(&["left"]))
            .contains(&Input::Down(doom::RSHIFT)));
    }

    #[test]
    fn taps_go_down_for_one_tic_once_a_press() {
        let mut k = Keyboard::new();
        k.press(&key("enter"));
        // Held or not, a tap is a tap.
        let held = plain(&["enter"]);
        assert_eq!(k.step(&held), vec![Input::Down(doom::ENTER)]);
        assert_eq!(k.step(&held), vec![Input::Up(doom::ENTER)]);
        assert_eq!(k.step(&held), vec![]);
        // "iddqd": the doubled letter is two taps, even within a tic.
        for c in ["i", "d", "d", "q", "d"] {
            k.press(&key(c));
        }
        let downs = k.step(&[]);
        assert_eq!(downs, b"iddqd".map(Input::Down).to_vec());
        assert_eq!(k.step(&[]).len(), 5, "let go on the next tic");
        assert!(!k.press(&key("pageup")), "not Doom's");
    }
}
