//! Pixel rectangles and points. On Windows these are the Win32 types, so platform
//! code passes them straight to the API; elsewhere they are plain structs with the
//! same fields.
#[cfg(windows)]
pub use windows::Win32::Foundation::{POINT, RECT};

#[cfg(not(windows))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[cfg(not(windows))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}
