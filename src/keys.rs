//! Modifier state shared by every keyboard path, so recording and matching agree.
use windows::Win32::UI::Input::KeyboardAndMouse::*;

/// RegisterHotKey-style modifier bits for the keys reported as held.
/// Either Windows key counts as Win; Ctrl/Shift/Alt use their side-agnostic codes.
pub fn held(down: impl Fn(VIRTUAL_KEY) -> bool) -> u32 {
    let mut m = 0;
    for (keys, bit) in [
        (&[VK_CONTROL][..], MOD_CONTROL),
        (&[VK_SHIFT][..], MOD_SHIFT),
        (&[VK_MENU][..], MOD_ALT),
        (&[VK_LWIN, VK_RWIN][..], MOD_WIN),
    ] {
        if keys.iter().any(|k| down(*k)) {
            m |= bit.0;
        }
    }
    m
}

/// What the keyboard says right now.
pub fn held_now() -> u32 {
    held(|v| unsafe { GetAsyncKeyState(v.0 as i32) } < 0)
}

pub fn is_modifier(vk: VIRTUAL_KEY) -> bool {
    matches!(
        vk,
        VK_SHIFT | VK_CONTROL | VK_MENU | VK_LSHIFT | VK_RSHIFT | VK_LCONTROL | VK_RCONTROL | VK_LMENU | VK_RMENU | VK_LWIN | VK_RWIN
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(keys: &'static [VIRTUAL_KEY]) -> impl Fn(VIRTUAL_KEY) -> bool {
        move |k| keys.contains(&k)
    }

    #[test]
    fn either_windows_key_is_win() {
        assert_eq!(held(only(&[VK_LWIN])), MOD_WIN.0);
        assert_eq!(held(only(&[VK_RWIN])), MOD_WIN.0);
        assert_eq!(held(only(&[VK_LWIN, VK_RWIN])), MOD_WIN.0);
        assert_eq!(held(only(&[VK_RWIN, VK_SHIFT])), (MOD_WIN | MOD_SHIFT).0);
    }

    #[test]
    fn existing_combinations_are_unchanged() {
        assert_eq!(held(only(&[])), 0);
        assert_eq!(held(only(&[VK_CONTROL, VK_SHIFT])), (MOD_CONTROL | MOD_SHIFT).0);
        assert_eq!(held(only(&[VK_MENU])), MOD_ALT.0);
        // Bits stored by earlier versions (Ctrl=2, Shift=4, Alt=1, Win=8) still match.
        assert_eq!(held(only(&[VK_CONTROL, VK_SHIFT])), 0x2 | 0x4);
        assert_eq!(held(only(&[VK_LWIN])), 0x8);
    }

    #[test]
    fn both_win_keys_are_modifiers() {
        assert!(is_modifier(VK_LWIN) && is_modifier(VK_RWIN));
        assert!(!is_modifier(VK_S) && !is_modifier(VK_SNAPSHOT));
    }
}
